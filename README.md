# cam

CodeAgent memory CLI. Index a project into a local SQLite code graph, then recall past solutions so an agent does not re-read or re-solve the same problem.

See [DESIGN.md](DESIGN.md) for the design and agent command sheet.

```bash
cargo install --path .
cam init
cam index
cam ls src/
cam read src/main.rs/main
cam ref main --dir in
cam recall "how does hybrid recall fuse BM25 and vectors"
cam add --summary "..." < notes.md
```
