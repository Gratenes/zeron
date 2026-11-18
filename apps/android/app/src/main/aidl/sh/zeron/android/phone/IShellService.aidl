package sh.zeron.android.phone;

/** Runs a fixed argv as the shell user (inside Shizuku's process). Never takes a shell string: callers build the argv. */
interface IShellService {
    /** Shizuku's reserved transaction for tearing the service down. */
    void destroy() = 16777114;

    /** Runs [argv], waits up to [timeoutMs], and returns "<exit code>\n<combined output, at most 64 KB>". */
    String run(in String[] argv, int timeoutMs) = 1;
}
