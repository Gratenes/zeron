package testderp

import (
	"bytes"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"
	"testing"

	"tailscale.com/tstest/integration"
	"tailscale.com/types/logger"
)

// TestTailcatFixture verifies the local DERP/STUN fixture and its HTTPS map.
// The Rust integration explicitly requests serving mode to keep this test-only
// endpoint alive until terminated. Ordinary `go test ./...` runs to completion.
func TestTailcatFixture(t *testing.T) {
	dm := integration.RunDERPAndSTUN(t, logger.Discard, "127.0.0.1")
	mapJSON, err := json.Marshal(dm)
	if err != nil {
		t.Fatal(err)
	}
	tlsServer := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write(mapJSON)
	}))
	defer tlsServer.Close()
	certPath := filepath.Join(t.TempDir(), "derp-map-ca.pem")
	cert := tlsServer.Certificate()
	if err := os.WriteFile(certPath, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: cert.Raw}), 0600); err != nil {
		t.Fatal(err)
	}
	descriptor, _ := json.Marshal(map[string]string{"map_url": tlsServer.URL, "cert_file": certPath})
	fmt.Println(string(descriptor))
	response, err := tlsServer.Client().Get(tlsServer.URL)
	if err != nil {
		t.Fatal(err)
	}
	body, err := io.ReadAll(response.Body)
	response.Body.Close()
	if err != nil || response.StatusCode != http.StatusOK || !bytes.Equal(body, mapJSON) {
		t.Fatalf("fixture map roundtrip: status=%d err=%v body=%q", response.StatusCode, err, body)
	}
	if os.Getenv("KRATOS_TAILCAT_FIXTURE_SERVE") != "1" {
		return
	}
	signals := make(chan os.Signal, 1)
	signal.Notify(signals, syscall.SIGINT, syscall.SIGTERM)
	defer signal.Stop(signals)
	<-signals
}
