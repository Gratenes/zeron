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
	public := key.Public().(ed25519.PublicKey)
	deviceID := deviceID(public)
	proof := ed25519.Sign(key, framed("kratos.peer-auth.invite-redeem.v1\x00", []byte{1}, []byte(invitation.Invite.ProfileID), []byte(invitation.Invite.InviteID), secret, public))
	redeem := map[string]any{
		"version": 1, "profileId": invitation.Invite.ProfileID, "inviteId": invitation.Invite.InviteID,
		"secret": invitation.Invite.Secret, "publicKey": base64url.EncodeToString(public),
		"signature": base64url.EncodeToString(proof), "displayName": displayName,
	}
	var result struct {
		ProfileID string `json:"profileId"`
		DeviceID  string `json:"deviceId"`
	}
	if err := c.postAuth("pair/redeem", redeem, &result); err != nil {
		return "", err
	}
	if result.ProfileID != invitation.Invite.ProfileID || result.DeviceID != deviceID {
		return "", errors.New("peer returned an invalid pairing identity")
	}
	return c.Authenticate(invitation.Invite.ProfileID)
}

// Authenticate renews a short-lived bearer using the persisted device key.
func (c *Client) Authenticate(profileID string) (string, error) {
	key, err := c.loadIdentity(profileID)
	if err != nil {
		return "", err
	}
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
	if err := os.MkdirAll(c.stateDir, 0700); err != nil {
		return nil, errors.New("could not create identity directory")
	}
	if err := os.Chmod(c.stateDir, 0700); err != nil {
		return nil, errors.New("could not secure identity directory")
	}
	file, err := os.CreateTemp(c.stateDir, ".device-key-*")
	if err != nil {
		return nil, errors.New("could not create device identity")
	}
	defer os.Remove(file.Name())
	if err := file.Chmod(0600); err != nil {
		_ = file.Close()
		return nil, errors.New("could not secure device identity")
	}
	if _, err = file.Write(key); err == nil {
		err = file.Sync()
	}
	closeErr := file.Close()
	if err == nil {
		err = closeErr
	}
	if err != nil {
		return nil, errors.New("could not save device identity")
	}
	if err := os.Rename(file.Name(), path); err != nil {
		return nil, errors.New("could not install device identity")
	}
	if dir, err := os.Open(c.stateDir); err == nil {
		_ = dir.Sync()
		_ = dir.Close()
	}
	return key, nil
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
