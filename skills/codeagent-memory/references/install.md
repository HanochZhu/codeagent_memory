# Install the CLI

Run this yourself, once, on a machine that already has Rust. The skill must not run it.

Official source (the project repository, no telemetry):

https://github.com/HanochZhu/codeagent_memory

```text
cargo install --git https://github.com/HanochZhu/codeagent_memory --locked
cam --help
```

That places `cam` in the Cargo bin directory (`~/.cargo/bin`, or `%USERPROFILE%\.cargo\bin` on Windows). If the shell cannot see it, add that directory to PATH and open a new terminal.

This skill package does not contain the binary. Do not download a script and pipe it into a shell.
