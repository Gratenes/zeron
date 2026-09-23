package tailcatnative

import (
	"bytes"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"sync"
	"time"
)

const maxAuthBody = 16 * 1024

var authHTTP = &http.Client{Timeout: 15 * time.Second}
var base64url = base64.RawURLEncoding
var identityMu sync.Mutex
var pairMu sync.Mutex
var errPeerForbidden = errors.New("peer pairing returned HTTP 403")

type pairingInvite struct {
	Version int    `json:"version"`
	Address string `json:"address"`
	DerpMap string `json:"derpMap"`
	Invite  struct {
		Version   int    `json:"version"`
		ProfileID string `json:"profileId"`
		InviteID  string `json:"inviteId"`
		Secret    string `json:"secret"`
	} `json:"invite"`
}

type authChallenge struct {
	ProfileID   string `json:"profileId"`
	DeviceID    string `json:"deviceId"`
	ChallengeID string `json:"challengeId"`
	Nonce       string `json:"nonce"`
}

type authSession struct {
	Token     string `json:"token"`
	ExpiresAt int64  `json:"expiresAt"`
	Principal struct {
		ProfileID string `json:"profileId"`
		DeviceID  string `json:"deviceId"`
	} `json:"principal"`
}

// Pair redeems a one-use invitation and returns an authenticated session as
// JSON. The device signing key is persisted before redemption so a crash does
// not lose the identity tied to a consumed invitation.
func (c *Client) Pair(invitationJSON, displayName string) (string, error) {
	pairMu.Lock()
	defer pairMu.Unlock()
	if len(invitationJSON) > maxAuthBody {
		return "", errors.New("invitation is too large")
	}
	var invitation pairingInvite
	if err := json.Unmarshal([]byte(invitationJSON), &invitation); err != nil || invitation.Version != 1 || invitation.Invite.Version != 1 || invitation.Invite.ProfileID == "" || invitation.Invite.InviteID == "" {
		return "", errors.New("invalid invitation")
	}
	secret, err := base64url.DecodeString(invitation.Invite.Secret)
	if err != nil || len(secret) != 32 {
		return "", errors.New("invalid invitation secret")
	}
	if len(displayName) > 128 {
		return "", errors.New("device name is too long")
	}
	key, err := c.identity(invitation.Invite.ProfileID)
	if err != nil {
		return "", err
	}
	pendingKey, pendingPath, err := c.pendingIdentity(invitation.Invite.ProfileID)
	if err != nil {
		return "", err
	}
	if pendingKey != nil {
		if session, err := c.authenticateKey(invitation.Invite.ProfileID, pendingKey); err == nil {
			if err := c.promotePending(invitation.Invite.ProfileID, pendingPath); err != nil {
				return "", err
			}
			return session, nil
		}
	}
	replacingIdentity := false
	var result struct {
		ProfileID string `json:"profileId"`
		DeviceID  string `json:"deviceId"`
	}
	redeem := func(key ed25519.PrivateKey) error {
		public := key.Public().(ed25519.PublicKey)
		proof := ed25519.Sign(key, framed("kratos.peer-auth.invite-redeem.v1\x00", []byte{1}, []byte(invitation.Invite.ProfileID), []byte(invitation.Invite.InviteID), secret, public))
		request := map[string]any{
			"version": 1, "profileId": invitation.Invite.ProfileID, "inviteId": invitation.Invite.InviteID,
			"secret": invitation.Invite.Secret, "publicKey": base64url.EncodeToString(public),
			"signature": base64url.EncodeToString(proof), "displayName": displayName,
		}
		return c.postAuth("pair/redeem", request, &result)
	}
	if err := redeem(key); errors.Is(err, errPeerForbidden) {
		replacingIdentity = true
		// A pending key survives a lost reply or process restart. Reuse it
		// until the peer definitively rejects that key too.
		if pendingKey == nil {
			pendingKey, pendingPath, err = c.newPendingIdentity(invitation.Invite.ProfileID)
			if err != nil {
				return "", err
			}
		}
		key = pendingKey
		err = redeem(key)
		if errors.Is(err, errPeerForbidden) {
			// The pending key itself was revoked. A new invitation can still
			// redeem a newly staged key.
			key, pendingPath, err = c.newPendingIdentity(invitation.Invite.ProfileID)
			if err != nil {
				return "", err
			}
			err = redeem(key)
		}
		if err != nil {
			if session, authErr := c.authenticatePending(invitation.Invite.ProfileID, key, pendingPath); authErr == nil {
				return session, nil
			}
			return "", err
		}
	} else if err != nil {
		return "", err
	}
	if result.ProfileID != invitation.Invite.ProfileID || result.DeviceID != deviceID(key.Public().(ed25519.PublicKey)) {
		if replacingIdentity {
			if session, err := c.authenticatePending(invitation.Invite.ProfileID, key, pendingPath); err == nil {
				return session, nil
			}
		}
		return "", errors.New("peer returned an invalid pairing identity")
	}
	if replacingIdentity {
		if err := c.promotePending(invitation.Invite.ProfileID, pendingPath); err != nil {
			return "", err
		}
	}
	return c.authenticateKey(invitation.Invite.ProfileID, key)
}

// Authenticate renews a short-lived bearer using the persisted device key.
func (c *Client) Authenticate(profileID string) (string, error) {
	pairMu.Lock()
	defer pairMu.Unlock()
	key, err := c.loadIdentity(profileID)
	if err != nil {
		return "", err
	}
	session, err := c.authenticateKey(profileID, key)
	if err == nil {
		return session, nil
	}
	pendingKey, pendingPath, pendingErr := c.pendingIdentity(profileID)
	if pendingErr != nil {
		return "", pendingErr
	}
	if pendingKey == nil {
		return "", err
	}
	session, pendingErr = c.authenticatePending(profileID, pendingKey, pendingPath)
	if pendingErr != nil {
		return "", pendingErr
	}
	return session, nil
}

func (c *Client) authenticatePending(profileID string, key ed25519.PrivateKey, pendingPath string) (string, error) {
	session, err := c.authenticateKey(profileID, key)
	if err != nil {
		return "", err
	}
	if err := c.promotePending(profileID, pendingPath); err != nil {
		return "", err
	}
	return session, nil
}

func (c *Client) authenticateKey(profileID string, key ed25519.PrivateKey) (string, error) {
	deviceID := deviceID(key.Public().(ed25519.PublicKey))
	var challenge authChallenge
	if err := c.postAuth("pair/challenge", map[string]string{"profileId": profileID, "deviceId": deviceID}, &challenge); err != nil {
		return "", err
	}
	nonce, err := base64url.DecodeString(challenge.Nonce)
	if err != nil || len(nonce) != 32 || challenge.ProfileID != profileID || challenge.DeviceID != deviceID || challenge.ChallengeID == "" {
		return "", errors.New("peer returned an invalid signing challenge")
	}
	signature := ed25519.Sign(key, framed("kratos.peer-auth.challenge.v1\x00", []byte(profileID), []byte(deviceID), []byte(challenge.ChallengeID), nonce))
	var session authSession
	if err := c.postAuth("pair/authenticate", map[string]string{
		"profileId": profileID, "deviceId": deviceID, "challengeId": challenge.ChallengeID,
		"nonce": challenge.Nonce, "signature": base64url.EncodeToString(signature),
	}, &session); err != nil {
		return "", err
	}
	if session.Token == "" || session.ExpiresAt <= time.Now().Unix() || session.Principal.ProfileID != profileID || session.Principal.DeviceID != deviceID {
		return "", errors.New("peer returned an invalid auth session")
	}
	encoded, err := json.Marshal(session)
	if err != nil {
		return "", err
	}
	return string(encoded), nil
}

func framed(domain string, fields ...[]byte) []byte {
	out := []byte(domain)
	for _, field := range fields {
		var size [4]byte
		binary.BigEndian.PutUint32(size[:], uint32(len(field)))
		out = append(out, size[:]...)
		out = append(out, field...)
	}
	return out
}

func deviceID(public ed25519.PublicKey) string {
	digest := sha256.Sum256(public)
	return "dev_" + base64url.EncodeToString(digest[:])
}

func (c *Client) identityPath(profileID string) (string, error) {
	if profileID == "" || c.stateDir == "" {
		return "", errors.New("missing device identity")
	}
	digest := sha256.Sum256([]byte(profileID))
	return filepath.Join(c.stateDir, "device-"+base64url.EncodeToString(digest[:])+".key"), nil
}

func (c *Client) loadIdentity(profileID string) (ed25519.PrivateKey, error) {
	path, err := c.identityPath(profileID)
	if err != nil {
		return nil, err
	}
	return loadIdentityFile(path)
}

func loadIdentityFile(path string) (ed25519.PrivateKey, error) {
	info, err := os.Lstat(path)
	if err != nil {
		return nil, errors.New("device identity is unavailable")
	}
	if !info.Mode().IsRegular() || (runtime.GOOS != "windows" && info.Mode().Perm() != 0600) {
		return nil, errors.New("device identity is not private")
	}
	key, err := os.ReadFile(path)
	if err != nil || len(key) != ed25519.PrivateKeySize {
		return nil, errors.New("device identity is invalid")
	}
	return ed25519.PrivateKey(key), nil
}

func (c *Client) pendingIdentity(profileID string) (ed25519.PrivateKey, string, error) {
	path, err := c.identityPath(profileID)
	if err != nil {
		return nil, "", err
	}
	pending := path + ".pending"
	if _, err := os.Lstat(pending); os.IsNotExist(err) {
		return nil, pending, nil
	} else if err != nil {
		return nil, "", err
	}
	key, err := loadIdentityFile(pending)
	return key, pending, err
}

func (c *Client) newPendingIdentity(profileID string) (ed25519.PrivateKey, string, error) {
	path, err := c.identityPath(profileID)
	if err != nil {
		return nil, "", err
	}
	_, key, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		return nil, "", err
	}
	staged, err := c.stageIdentity(key)
	if err != nil {
		return nil, "", err
	}
	defer os.Remove(staged)
	pending := path + ".pending"
	if err := c.installIdentity(staged, pending); err != nil {
		return nil, "", err
	}
	return key, pending, nil
}

func (c *Client) promotePending(profileID, pending string) error {
	path, err := c.identityPath(profileID)
	if err != nil {
		return err
	}
	return c.installIdentity(pending, path)
}

func (c *Client) identity(profileID string) (ed25519.PrivateKey, error) {
	identityMu.Lock()
	defer identityMu.Unlock()
	path, err := c.identityPath(profileID)
	if err != nil {
		return nil, err
	}
	if _, err := os.Lstat(path); err == nil {
		return c.loadIdentity(profileID)
	} else if !os.IsNotExist(err) {
		return nil, err
	}
	_, key, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		return nil, err
	}
	staged, err := c.stageIdentity(key)
	if err != nil {
		return nil, err
	}
	defer os.Remove(staged)
	if err := c.installIdentity(staged, path); err != nil {
		return nil, err
	}
	return key, nil
}

func (c *Client) stageIdentity(key ed25519.PrivateKey) (string, error) {
	if err := os.MkdirAll(c.stateDir, 0700); err != nil {
		return "", errors.New("could not create identity directory")
	}
	if err := os.Chmod(c.stateDir, 0700); err != nil {
		return "", errors.New("could not secure identity directory")
	}
	file, err := os.CreateTemp(c.stateDir, ".device-key-*")
	if err != nil {
		return "", errors.New("could not create device identity")
	}
	staged := file.Name()
	valid := false
	defer func() {
		if !valid {
			_ = os.Remove(staged)
		}
	}()
	if err := file.Chmod(0600); err != nil {
		_ = file.Close()
		return "", errors.New("could not secure device identity")
	}
	if _, err = file.Write(key); err == nil {
		err = file.Sync()
	}
	closeErr := file.Close()
	if err == nil {
		err = closeErr
	}
	if err != nil {
		return "", errors.New("could not save device identity")
	}
	valid = true
	return staged, nil
}

func (c *Client) installIdentity(staged, path string) error {
	if err := os.Rename(staged, path); err != nil {
		return errors.New("could not install device identity")
	}
	if dir, err := os.Open(c.stateDir); err == nil {
		_ = dir.Sync()
		_ = dir.Close()
	}
	return nil
}

func (c *Client) postAuth(path string, payload, result any) error {
	body, err := json.Marshal(payload)
	if err != nil {
		return err
	}
	request, err := http.NewRequest(http.MethodPost, c.url+"/"+path, bytes.NewReader(body))
	if err != nil {
		return err
	}
	request.Header.Set("Content-Type", "application/json")
	response, err := authHTTP.Do(request)
	if err != nil {
		return fmt.Errorf("peer pairing request failed: %w", err)
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		if response.StatusCode == http.StatusForbidden {
			return errPeerForbidden
		}
		return fmt.Errorf("peer pairing returned HTTP %d", response.StatusCode)
	}
	limited := io.LimitReader(response.Body, maxAuthBody+1)
	data, err := io.ReadAll(limited)
	if err != nil {
		return err
	}
	if len(data) > maxAuthBody {
		return errors.New("peer pairing response is too large")
	}
	if err := json.Unmarshal(data, result); err != nil {
		return errors.New("peer pairing response is invalid")
	}
	return nil
}
