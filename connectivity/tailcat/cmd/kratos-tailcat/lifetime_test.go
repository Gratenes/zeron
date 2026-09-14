package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
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

// An intermediate owner exits without closing Child/stdin or running deferred
// cleanup, just like a native UI exit or crash. The adapter must release its
// handles without assistance from the test process (which is not its parent).
func TestManagedAdapterExitsWithParent(t *testing.T) {
	t.Setenv("IN_TS_TEST", "true")
	dm := integration.RunDERPAndSTUN(t, logger.Discard, "127.0.0.1")
	mapJSON, _ := json.Marshal(dm)
	mapServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write(mapJSON)
	}))
	defer mapServer.Close()
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte("alive"))
	}))
	defer backend.Close()
	server, err := native.StartServerCLI(backend.Listener.Addr().String(), filepath.Join(t.TempDir(), "server.key"), mapServer.URL, 1)
	if err != nil {
		t.Fatal(err)
	}
	defer server.Close()

	for _, mode := range []string{"connect", "serve"} {
		for _, crash := range []bool{false, true} {
			t.Run(fmt.Sprintf("%s/crash=%v", mode, crash), func(t *testing.T) {
				dir := t.TempDir()
				args := []string{mode, "--state", filepath.Join(dir, "state.key")}
				if mode == "connect" {
					config := filepath.Join(dir, "connect.json")
					data, _ := json.Marshal(connectConfig{Address: server.Address()})
					if err := os.WriteFile(config, data, 0600); err != nil {
						t.Fatal(err)
					}
					args = append(args, "--config", config)
				} else {
					args = append(args, "--target", backend.Listener.Addr().String(), "--derp-map", mapServer.URL, "--region", "1")
				}
				exe, err := os.Executable()
				if err != nil {
					t.Fatal(err)
				}
				owner := exec.Command(exe, append([]string{"-test.run=^TestAdapterOwnerProcess$", "--"}, args...)...)
				owner.Env = append(os.Environ(), "KRATOS_OWNER_HELPER=1")
				input, err := owner.StdinPipe()
				if err != nil {
					t.Fatal(err)
				}
				defer input.Close()
				output, err := owner.StdoutPipe()
				if err != nil {
					t.Fatal(err)
				}
				// Both owner and adapter inherit this write end. EOF proves the
				// orphan adapter exited too, including on Windows without /proc.
				errRead, errWrite, err := os.Pipe()
				if err != nil {
					t.Fatal(err)
				}
				defer errRead.Close()
				owner.Stderr = errWrite
				if err := owner.Start(); err != nil {
					errWrite.Close()
					t.Fatal(err)
				}
				_ = errWrite.Close()
				defer func() { _ = owner.Process.Kill(); _ = owner.Wait() }()
				exited := make(chan struct{})
				go func() { _, _ = io.Copy(io.Discard, errRead); close(exited) }()
				ready := make(chan ownerReady, 1)
				go func() {
					var value ownerReady
					_ = json.NewDecoder(output).Decode(&value)
					ready <- value
				}()
				var value ownerReady
				select {
				case value = <-ready:
					if value.PID == 0 {
						t.Fatal("owner did not publish adapter readiness")
					}
				case <-time.After(25 * time.Second):
					t.Fatal("adapter startup timed out")
				}
				cleanExit := false
				defer func() {
					if !cleanExit {
						if p, err := os.FindProcess(value.PID); err == nil {
							_ = p.Kill()
						}
					}
				}()
				var adapter struct {
					URL     string `json:"url"`
					Address string `json:"address"`
				}
				if err := json.Unmarshal(value.Ready, &adapter); err != nil {
					t.Fatal(err)
				}
				if mode == "connect" {
					httpClient := &http.Client{Timeout: 5 * time.Second}
					response, err := httpClient.Get(adapter.URL)
					if err != nil {
						t.Fatal(err)
					}
					body, err := io.ReadAll(response.Body)
					response.Body.Close()
					httpClient.CloseIdleConnections()
					if err != nil || string(body) != "alive" {
						t.Fatal("adapter failed while owner was alive")
					}
				} else if adapter.Address == "" {
					t.Fatal("missing server readiness")
				}
				deadline := time.Now().Add(5 * time.Second)
				if crash {
					_ = owner.Process.Kill()
				} else {
					_ = input.Close()
				}
				select {
				case <-exited:
					cleanExit = true
				case <-time.After(time.Until(deadline)):
					t.Fatal("adapter survived owner exit")
				}
				if mode == "connect" {
					// stderr EOF can precede kernel socket teardown on Windows.
					// Keep the same overall exit deadline: require an actual
					// successful exclusive bind, not just a dead process/closed pipe.
					for {
						listener, err := net.Listen("tcp4", strings.TrimPrefix(adapter.URL, "http://"))
						if err == nil {
							listener.Close()
							break
						}
						if time.Now().After(deadline) {
							t.Fatalf("adapter left loopback port occupied after exit deadline: %v", err)
						}
						time.Sleep(10 * time.Millisecond)
					}
				}
			})
		}
	}
}

type ownerReady struct {
	PID   int
	Ready json.RawMessage
}

func TestAdapterOwnerProcess(t *testing.T) {
	if os.Getenv("KRATOS_OWNER_HELPER") != "1" {
		return
	}
	for i, arg := range os.Args {
		if arg != "--" {
			continue
		}
		exe, _ := os.Executable()
		child := exec.Command(exe, append([]string{"-test.run=^TestConnectHelperProcess$", "--"}, os.Args[i+1:]...)...)
		child.Env = append(os.Environ(), "KRATOS_TAILCAT_HELPER=1", "KRATOS_TAILCAT_PARENT_PIPE=1")
		pipe, err := child.StdinPipe()
		if err != nil {
			os.Exit(3)
		}
		stdout, err := child.StdoutPipe()
		if err != nil {
			os.Exit(3)
		}
		child.Stderr = os.Stderr
		if child.Start() != nil {
			os.Exit(3)
		}
		line, err := bufio.NewReader(stdout).ReadBytes('\n')
		if err != nil {
			os.Exit(3)
		}
		_ = json.NewEncoder(os.Stdout).Encode(ownerReady{PID: child.Process.Pid, Ready: line})
		_, _ = io.Copy(io.Discard, os.Stdin)
		runtime.KeepAlive(pipe)
		// Deliberately no Close/Wait/defer: the kernel closes the owner's pipe.
		os.Exit(0)
	}
	os.Exit(3)
}
