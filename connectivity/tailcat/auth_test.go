package tailcatnative

import (
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sync/atomic"
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

func TestPairRevokedIdentityUsesFreshKeyOnlyAfterAcceptedRedeem(t *testing.T) {
	secret := make([]byte, 32)
	if _, err := rand.Read(secret); err != nil {
		t.Fatal(err)
	}
	var original, active ed25519.PublicKey
	redeems := make(chan string, 5)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		switch r.URL.Path {
		case "/pair/redeem":
			var request struct {
				ProfileID, InviteID, Secret, PublicKey, Signature string
			}
			if err := json.NewDecoder(r.Body).Decode(&request); err != nil {
				t.Error(err)
				w.WriteHeader(http.StatusBadRequest)
				return
			}
			public, err := base64url.DecodeString(request.PublicKey)
			signature, signatureErr := base64url.DecodeString(request.Signature)
			if err != nil || signatureErr != nil || request.ProfileID != "profile" || request.Secret != base64url.EncodeToString(secret) || !ed25519.Verify(public, framed("kratos.peer-auth.invite-redeem.v1\x00", []byte{1}, []byte("profile"), []byte(request.InviteID), secret, public), signature) {
				t.Error("invalid redeem proof")
				w.WriteHeader(http.StatusBadRequest)
				return
			}
			redeems <- request.PublicKey
			if request.InviteID == "first" {
				original = public
			} else if string(public) == string(original) {
				w.WriteHeader(http.StatusForbidden)
				return
			} else if request.InviteID == "rejected" {
				w.WriteHeader(http.StatusBadRequest)
				return
			}
			active = public
			_ = json.NewEncoder(w).Encode(map[string]string{"profileId": "profile", "deviceId": deviceID(public)})
		case "/pair/challenge":
			_ = json.NewEncoder(w).Encode(map[string]string{"profileId": "profile", "deviceId": deviceID(active), "challengeId": "challenge", "nonce": base64url.EncodeToString(secret)})
		case "/pair/authenticate":
			_ = json.NewEncoder(w).Encode(map[string]any{"token": "token", "expiresAt": time.Now().Unix() + 900, "principal": map[string]string{"profileId": "profile", "deviceId": deviceID(active)}})
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}))
	defer server.Close()
	client := &Client{url: server.URL, stateDir: t.TempDir()}
	invite := func(id string) string {
		data, _ := json.Marshal(map[string]any{"version": 1, "invite": map[string]any{"version": 1, "profileId": "profile", "inviteId": id, "secret": base64url.EncodeToString(secret)}})
		return string(data)
	}
	if _, err := client.Pair(invite("first"), "Phone"); err != nil {
		t.Fatal(err)
	}
	oldKey, err := client.loadIdentity("profile")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := client.Pair(invite("rejected"), "Phone"); err == nil {
		t.Fatal("rejected replacement was accepted")
	}
	stillOld, err := client.loadIdentity("profile")
	if err != nil || string(stillOld) != string(oldKey) {
		t.Fatal("failed repair replaced the saved identity")
	}
	if _, err := client.Pair(invite("repair"), "Phone"); err != nil {
		t.Fatal(err)
	}
	newKey, err := client.loadIdentity("profile")
	if err != nil || string(newKey) == string(oldKey) {
		t.Fatal("successful repair did not save a fresh identity")
	}
	if len(redeems) != 5 {
		t.Fatalf("got %d redeem requests, want 5", len(redeems))
	}
	oldPublic := base64url.EncodeToString(oldKey.Public().(ed25519.PublicKey))
	newPublic := base64url.EncodeToString(newKey.Public().(ed25519.PublicKey))
	for i, want := range []string{oldPublic, oldPublic, newPublic, oldPublic, newPublic} {
		got := <-redeems
		if got != want {
			t.Fatalf("redeem %d used the wrong identity", i+1)
		}
	}
	if _, err := client.Authenticate("profile"); err != nil {
		t.Fatal(err)
	}
}

func TestPairRecoversCommittedRedeemAfterLostReply(t *testing.T) {
	for _, delayed := range []bool{false, true} {
		t.Run(map[bool]string{false: "immediate", true: "after restart"}[delayed], func(t *testing.T) {
			secret := make([]byte, 32)
			var active ed25519.PublicKey
			var blockProof atomic.Bool
			blockProof.Store(delayed)
			redeems := 0
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.Header().Set("Content-Type", "application/json")
				switch r.URL.Path {
				case "/pair/redeem":
					var request struct{ PublicKey string }
					if err := json.NewDecoder(r.Body).Decode(&request); err != nil {
						t.Error(err)
						return
					}
					public, err := base64url.DecodeString(request.PublicKey)
					if err != nil {
						t.Error(err)
						return
					}
					redeems++
					if redeems == 1 {
						active = public
						_ = json.NewEncoder(w).Encode(map[string]string{"profileId": "profile", "deviceId": deviceID(public)})
					} else if string(public) == string(active) {
						w.WriteHeader(http.StatusForbidden)
					} else {
						active = public // The peer commits before its response is lost.
						_, _ = w.Write([]byte("{"))
					}
				case "/pair/challenge":
					var request struct{ DeviceID string }
					_ = json.NewDecoder(r.Body).Decode(&request)
					if request.DeviceID != deviceID(active) {
						w.WriteHeader(http.StatusForbidden)
					} else if blockProof.Load() && redeems > 1 {
						w.WriteHeader(http.StatusServiceUnavailable)
					} else {
						_ = json.NewEncoder(w).Encode(map[string]string{"profileId": "profile", "deviceId": request.DeviceID, "challengeId": "challenge", "nonce": base64url.EncodeToString(secret)})
					}
				case "/pair/authenticate":
					_ = json.NewEncoder(w).Encode(map[string]any{"token": "token", "expiresAt": time.Now().Unix() + 900, "principal": map[string]string{"profileId": "profile", "deviceId": deviceID(active)}})
				default:
					w.WriteHeader(http.StatusNotFound)
				}
			}))
			defer server.Close()
			client := &Client{url: server.URL, stateDir: t.TempDir()}
			invite := func(id string) string {
				data, _ := json.Marshal(map[string]any{"version": 1, "invite": map[string]any{"version": 1, "profileId": "profile", "inviteId": id, "secret": base64url.EncodeToString(secret)}})
				return string(data)
			}
			if _, err := client.Pair(invite("first"), "Phone"); err != nil {
				t.Fatal(err)
			}
			path, _ := client.identityPath("profile")
			oldKey, _ := os.ReadFile(path)
			_, err := client.Pair(invite("repair"), "Phone")
			if delayed && err == nil || !delayed && err != nil {
				t.Fatalf("lost redeem reply: %v", err)
			}
			if delayed {
				stillOld, _ := os.ReadFile(path)
				if string(stillOld) != string(oldKey) {
					t.Fatal("uncertain redeem replaced the old identity")
				}
				if _, err := os.Stat(path + ".pending"); err != nil {
					t.Fatal("pending identity was lost:", err)
				}
				blockProof.Store(false)
				client = &Client{url: server.URL, stateDir: client.stateDir}
				if _, err := client.Authenticate("profile"); err != nil {
					t.Fatal("restored pending identity did not authenticate:", err)
				}
			}
			newKey, _ := os.ReadFile(path)
			if string(newKey) == string(oldKey) {
				t.Fatal("accepted replacement was not installed")
			}
			if _, err := os.Stat(path + ".pending"); !os.IsNotExist(err) {
				t.Fatalf("pending identity remains after promotion: %v", err)
			}
			if redeems != 3 {
				t.Fatalf("got %d redeems, want 3", redeems)
			}
		})
	}
}

func TestFirstPairRecoversLostRedeemReplyOnRetry(t *testing.T) {
	for _, delayed := range []bool{false, true} {
		t.Run(map[bool]string{false: "immediate proof", true: "retry after restart"}[delayed], func(t *testing.T) {
			testFirstPairLostReply(t, delayed)
		})
	}
}

func testFirstPairLostReply(t *testing.T, delayed bool) {
	secret := make([]byte, 32)
	var blockProof atomic.Bool
	blockProof.Store(delayed)
	var redeems atomic.Int32
	var accepted atomic.Value
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		switch r.URL.Path {
		case "/pair/redeem":
			var request struct{ InviteID, PublicKey, Signature string }
			if err := json.NewDecoder(r.Body).Decode(&request); err != nil {
				t.Error(err)
				return
			}
			public, publicErr := base64url.DecodeString(request.PublicKey)
			signature, signatureErr := base64url.DecodeString(request.Signature)
			if publicErr != nil || signatureErr != nil || !ed25519.Verify(public, framed("kratos.peer-auth.invite-redeem.v1\x00", []byte{1}, []byte("profile"), []byte(request.InviteID), secret, public), signature) {
				t.Error("invalid redeem proof")
				w.WriteHeader(http.StatusUnauthorized)
				return
			}
			if redeems.Add(1) != 1 {
				w.WriteHeader(http.StatusConflict)
				return
			}
			accepted.Store(request.PublicKey) // Peer commits, but the response is lost.
			_, _ = w.Write([]byte("{"))
		case "/pair/challenge":
			if blockProof.Load() {
				w.WriteHeader(http.StatusServiceUnavailable)
				return
			}
			public, _ := base64url.DecodeString(accepted.Load().(string))
			var request struct{ DeviceID string }
			_ = json.NewDecoder(r.Body).Decode(&request)
			if request.DeviceID != deviceID(public) {
				w.WriteHeader(http.StatusForbidden)
				return
			}
			_ = json.NewEncoder(w).Encode(map[string]string{"profileId": "profile", "deviceId": request.DeviceID, "challengeId": "challenge", "nonce": base64url.EncodeToString(secret)})
		case "/pair/authenticate":
			public, _ := base64url.DecodeString(accepted.Load().(string))
			var request struct{ Signature string }
			_ = json.NewDecoder(r.Body).Decode(&request)
			signature, _ := base64url.DecodeString(request.Signature)
			if !ed25519.Verify(public, framed("kratos.peer-auth.challenge.v1\x00", []byte("profile"), []byte(deviceID(public)), []byte("challenge"), secret), signature) {
				w.WriteHeader(http.StatusUnauthorized)
				return
			}
			_ = json.NewEncoder(w).Encode(map[string]any{"token": "token", "expiresAt": time.Now().Unix() + 900, "principal": map[string]string{"profileId": "profile", "deviceId": deviceID(public)}})
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}))
	defer server.Close()
	client := &Client{url: server.URL, stateDir: t.TempDir()}
	invitation, _ := json.Marshal(map[string]any{"version": 1, "invite": map[string]any{"version": 1, "profileId": "profile", "inviteId": "first", "secret": base64url.EncodeToString(secret)}})
	_, pairErr := client.Pair(string(invitation), "Phone")
	if delayed && pairErr == nil || !delayed && pairErr != nil {
		t.Fatalf("unexpected first pair result: %v", pairErr)
	}
	path, _ := client.identityPath("profile")
	primary, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !delayed {
		if _, err := os.Stat(path + ".pending"); !os.IsNotExist(err) {
			t.Fatalf("pending key remains after immediate proof: %v", err)
		}
		if redeems.Load() != 1 {
			t.Fatalf("consumed invitation was redeemed %d times", redeems.Load())
		}
		return
	}
	pending, err := os.ReadFile(path + ".pending")
	if err != nil || string(primary) != string(pending) {
		t.Fatal("uncertain first pairing did not retain its key")
	}
	blockProof.Store(false)
	client = &Client{url: server.URL, stateDir: client.stateDir}
	if _, err := client.Pair(string(invitation), "Phone"); err != nil {
		t.Fatal("same invitation could not restore pairing:", err)
	}
	if redeems.Load() != 1 {
		t.Fatalf("consumed invitation was redeemed %d times", redeems.Load())
	}
	if _, err := os.Stat(path + ".pending"); !os.IsNotExist(err) {
		t.Fatalf("pending key remains after proof: %v", err)
	}
}
