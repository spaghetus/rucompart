RUCOMPART
====

*(This demo is best viewed using "Presenterm" in an advanced terminal like `kitty`)*

<!-- pause -->
<!-- font_size: 2 -->
In short:
<!-- font_size: 1 -->

```file +auto_exec:rust-script
path: front-demo.rs
language: rust
```

<!-- end_slide -->

Bill of Materials
====
<!-- font_size: 2 -->
* Rust, a modern systems programming language
* tokio, an event-loop-based asynchronous runtime
* tarpc, an RPC framework using macros to define commands without leaving Rust

<!-- end_slide -->

Fork Demo
===

```bash +exec_replace +pty:standby:80:20
watch -n0.1 pstree $PPID -g3 -s rs &
sleep 20s
kill %1
```

```file +exec_replace:rust-script +pty:standby:80:20 +validate
path: fork-demo.rs
language: rust
```

<!-- end_slide -->

TCP Demo
===

```bash +exec_replace +pty:standby:80:20
watch 'ss -tnap | grep 1234'
sleep 20s
kill %1
```

```file +exec_replace:rust-script +pty:standby:80:10 +validate
path: tcp-demo-host.rs
language: rust
```

```file +exec_replace:rust-script +pty:standby:80:10
path: tcp-demo-guest.rs
language: rust
```