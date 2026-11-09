# personal builds (Gratenes/zeron)

This orphan branch only holds `.github/workflows/personal-builds.yml`. It shares
no history with upstream, so it can never be part of a PR. It is the fork's
default branch only so `workflow_dispatch` works. Never push to `main` here.

Start a build of any fork branch or SHA:

    gh workflow run personal-builds.yml -R Gratenes/zeron -f ref=<branch> [-f targets=linux|windows|both]

Download (no compiling on the machines):

    gh run download <run-id> -R Gratenes/zeron -n zeron-linux-server   # ./zeron-<sha>
    gh run download <run-id> -R Gratenes/zeron -n zeron-windows-pc     # zeron-zgx.exe, zeron-engine.exe

Artifacts are public and kept 7 days. No secrets, ever.
