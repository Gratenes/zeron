package tailcatnative

import (
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"
	"time"
)

func TestPairAndRestoreAuthenticate(t *testing.T) {
	secret := make([]byte, 32)
	if _, err := rand.Read(secret); err != nil {
		t.Fatal(err)
	}
	var public ed25519.PublicKey
	var id string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		switch r.URL.Path {
		case "/pair/redeem":
			var proof struct {
				ProfileID string `json:"profileId"`
				InviteID  string `json:"inviteId"`
				Secret    string `json:"secret"`
				PublicKey string `json:"publicKey"`
				Signature string `json:"signature"`
			}
			if err := json.NewDecoder(r.Body).Decode(&proof); err != nil {
				t.Error(err)
				return
			}
			public, _ = base64url.DecodeString(proof.PublicKey)
			signature, _ := base64url.DecodeString(proof.Signature)
			if proof.ProfileID != "profile" || proof.InviteID != "invite" || proof.Secret != base64url.EncodeToString(secret) || !ed25519.Verify(public, framed("kratos.peer-auth.invite-redeem.v1\x00", []byte{1}, []byte("profile"), []byte("invite"), secret, public), signature) {
				t.Error("invalid redeem proof")
			}
			id = deviceID(public)
			_, _ = w.Write([]byte(`{"profileId":"profile","deviceId":"` + id + `"}`))
		case "/pair/challenge":
			_, _ = w.Write([]byte(`{"profileId":"profile","deviceId":"` + id + `","challengeId":"challenge","nonce":"` + base64url.EncodeToString(secret) + `"}`))
		case "/pair/authenticate":
			var proof struct {
				Signature string `json:"signature"`
			}
			if err := json.NewDecoder(r.Body).Decode(&proof); err != nil {
				t.Error(err)
				return
			}
			signature, _ := base64url.DecodeString(proof.Signature)
			if !ed25519.Verify(public, framed("kratos.peer-auth.challenge.v1\x00", []byte("profile"), []byte(id), []byte("challenge"), secret), signature) {
				t.Error("invalid challenge proof")
			}
			response, _ := json.Marshal(map[string]any{"token": "token", "expiresAt": time.Now().Unix() + 900, "principal": map[string]string{"profileId": "profile", "deviceId": id}})
			_, _ = w.Write(response)
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}))
	defer server.Close()
	client := &Client{url: server.URL, stateDir: t.TempDir()}
	invitation, _ := json.Marshal(map[string]any{"version": 1, "address": "tailcat-address", "invite": map[string]any{"version": 1, "profileId": "profile", "inviteId": "invite", "secret": base64url.EncodeToString(secret)}})
	if _, err := client.Pair(string(invitation), "Phone"); err != nil {
		t.Fatal(err)
	}
	assertPrivateModeOnPOSIX(t, filepath.Join(client.stateDir, "device-"+base64url.EncodeToString(sha256Bytes([]byte("profile")))+".key"))
	if _, err := client.Authenticate("profile"); err != nil {
		t.Fatal(err)
	}
}

func sha256Bytes(value []byte) []byte { digest := sha256.Sum256(value); return digest[:] }
