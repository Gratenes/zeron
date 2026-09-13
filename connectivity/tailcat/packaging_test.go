package tailcatnative

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestPackagingIncludesAdjacentCompanion(t *testing.T) {
	root := filepath.Clean(filepath.Join("..", ".."))
	checks := map[string][]string{
		"scripts/package-linux.sh": {
			`install -m 755 "$TAILCAT_BIN" "$STAGE/kratos-tailcat"`,
			`install -Dm755 "$HERE/kratos-tailcat" "$HOME/.local/bin/kratos-tailcat"`,
		},
		"scripts/package-macos.sh": {
			`"$APP/Contents/MacOS/kratos-tailcat"`,
			"Nested executable code must be signed before the enclosing application",
		},
		"scripts/package-windows.ps1": {
			"Join-Path $stage 'kratos-tailcat.exe'",
			"companion = @{ file = 'kratos-tailcat.exe'",
			"sha256 = $companionHash",
		},
	}
	for name, required := range checks {
		data, err := os.ReadFile(filepath.Join(root, name))
		if err != nil {
			t.Fatal(err)
		}
		for _, text := range required {
			if !strings.Contains(string(data), text) {
				t.Errorf("%s does not contain %q", name, text)
			}
		}
	}
}
