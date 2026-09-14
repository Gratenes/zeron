package main

import (
	"bufio"
	"bytes"
	"encoding/json"

	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"

	native "github.com/kratos-ai/kratos/connectivity/tailcat"
	"tailscale.com/tstest/integration"
	"tailscale.com/types/logger"
)

func TestLoadConnectConfig(t *testing.T) {
	path := filepath.Join(t.TempDir(), "connect.json")
	if err := os.WriteFile(path, []byte(`{"address":"tc-secret"}`), 0600); err != nil {
		t.Fatal(err)
	}
	config, err := loadConnectConfig(path)
	if err != nil {
		t.Fatal(err)
	}
	if config.Address != "tc-secret" {
		t.Fatalf("address = %q", config.Address)
	}
}

func TestLoadConnectConfigFailsClosed(t *testing.T) {
	for _, test := range []struct {
		name string
		body string
	}{
		{"malformed", `{`},
		{"unknown field", `{"address":"tc-secret","extra":true}`},
		{"trailing JSON", `{"address":"tc-secret"}{}`},
		{"missing address", `{}`},
	} {
		t.Run(test.name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "connect.json")
			if err := os.WriteFile(path, []byte(test.body), 0600); err != nil {
				t.Fatal(err)
			}
			assertConnectConfigRejectedWithoutSecret(t, path)
		})
	}
}

func TestLoadConnectConfigRejectsUnsafeFilesystemObjects(t *testing.T) {
	dir := t.TempDir()
	assertConnectConfigRejectedWithoutSecret(t, dir)

	oversized := filepath.Join(dir, "oversized.json")
	body := append([]byte(`{"address":"tc-secret","padding":"`), bytes.Repeat([]byte("x"), maxConnectConfigSize)...)
	body = append(body, []byte(`"}`)...)
	if err := os.WriteFile(oversized, body, 0600); err != nil {
		t.Fatal(err)
	}
	assertConnectConfigRejectedWithoutSecret(t, oversized)
}

func TestLoadConnectConfigRejectsWidePermissionsOnPOSIX(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Windows authorization is enforced by ACLs, not POSIX mode bits")
	}
	path := filepath.Join(t.TempDir(), "connect.json")
	if err := os.WriteFile(path, []byte(`{"address":"tc-secret"}`), 0644); err != nil {
		t.Fatal(err)
	}
	assertConnectConfigRejectedWithoutSecret(t, path)
}

func assertConnectConfigRejectedWithoutSecret(t *testing.T, path string) {
	t.Helper()
	if _, err := loadConnectConfig(path); err == nil {
		t.Fatal("config accepted")
	} else if strings.Contains(err.Error(), "tc-secret") {
		t.Fatal("diagnostic leaked address")
	}
}

func TestConnectRejectsAddressArgument(t *testing.T) {
	err := runConnect([]string{"--address", "tc-secret", "--state", "state"})
	if err == nil || strings.Contains(err.Error(), "tc-secret") {
		t.Fatalf("error = %v", err)
	}
}

func TestConnectSecretAbsentFromProcessArgv(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("/proc argv assertion is Linux-specific")
	}
	t.Setenv("IN_TS_TEST", "true")
	dm := integration.RunDERPAndSTUN(t, logger.Discard, "127.0.0.1")
	mapJSON, err := json.Marshal(dm)
	if err != nil {
		t.Fatal(err)
	}
	mapServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write(mapJSON)
	}))
	defer mapServer.Close()
	backend, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer backend.Close()
	server, err := native.StartServerCLI(backend.Addr().String(), filepath.Join(t.TempDir(), "server.key"), mapServer.URL, 1)
	if err != nil {
		t.Fatal(err)
	}
	defer server.Close()

	dir := t.TempDir()
	configPath := filepath.Join(dir, "connect.json")
	secret := server.Address()
	configJSON, _ := json.Marshal(connectConfig{Address: secret})
	if err := os.WriteFile(configPath, configJSON, 0600); err != nil {
		t.Fatal(err)
	}
	testBinary, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(testBinary, "-test.run=^TestConnectHelperProcess$", "--", "connect", "--config", configPath, "--state", filepath.Join(dir, "client.key"))
	cmd.Env = append(os.Environ(), "KRATOS_TAILCAT_HELPER=1")
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	defer func() { _ = cmd.Process.Kill(); _ = cmd.Wait() }()
	ready := make(chan string, 1)
	go func() {
		scanner := bufio.NewScanner(stdout)
		if scanner.Scan() {
			ready <- scanner.Text()
			return
		}
		ready <- ""
	}()
	select {
	case line := <-ready:
		if !strings.Contains(line, `"url":"http://127.0.0.1:`) {
			t.Fatalf("readiness = %q; stderr=%s", line, stderr.String())
		}
	case <-time.After(20 * time.Second):
		t.Fatalf("connect readiness timed out; stderr=%s", stderr.String())
	}
	argv, err := os.ReadFile(filepath.Join("/proc", fmt.Sprint(cmd.Process.Pid), "cmdline"))
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(argv, []byte(secret)) {
		t.Fatal("Tailcat address leaked into process argv")
	}
	if !bytes.Contains(argv, []byte(configPath)) {
		t.Fatal("process argv does not contain config path")
	}
	if err := cmd.Process.Signal(os.Interrupt); err != nil {
		t.Fatal(err)
	}
	if err := cmd.Wait(); err != nil {
		t.Fatalf("helper exit: %v; stderr=%s", err, stderr.String())
	}
}

func TestConnectHelperProcess(t *testing.T) {
	if os.Getenv("KRATOS_TAILCAT_HELPER") != "1" {
		return
	}
	for i, arg := range os.Args {
		if arg == "--" {
			if err := run(os.Args[i+1:]); err != nil {
				fmt.Fprintln(os.Stderr, err)
				os.Exit(2)
			}
			return
		}
	}
	os.Exit(2)
}
