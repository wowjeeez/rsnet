# tsnet Production-Ready Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix all 16 bugs to make the tsnet Rust FFI crate production-ready.

**Architecture:** The crate wraps Tailscale's `libtailscale` C library (included as a git submodule) via Rust FFI using `bindgen`. The Rust layer adds a `mio`-based event loop for non-blocking I/O on socketpair fds returned by the Go/C library. Fixes span four areas: build system, public API surface, FFI correctness, and resource/lifecycle management.

**Tech Stack:** Rust (edition 2024), Go (c-archive build mode), bindgen, mio, libc

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `build.rs` | Modify | Fix paths, add -o flag, add link-lib directive |
| `Cargo.toml` | Modify | Remove unused cmake dep |
| `src/lib.rs` | Modify | Add pub re-exports |
| `src/glue/mod.rs` | Modify | Make server module pub |
| `src/glue/server.rs` | Modify (heavy) | Fix all FFI bugs, add Drop, add shutdown, add getips, add set_hostname |
| `src/glue/as_c_ptr.rs` | Delete | Dead code with memory leak |
| `src/vendor/mod.rs` | No change | Already correct |

---

### Task 1: Fix Build System (B5, B6, B11)

**Files:**
- Modify: `build.rs`
- Modify: `Cargo.toml`

**Context:** The build script uses relative paths that break when this crate is used as a dependency. It doesn't specify an output archive name or tell cargo what library to link. The `cmake` build-dep is unused.

- [ ] **Step 1: Fix build.rs — absolute paths, output name, link directive**

Replace the entire `build.rs` with:

```rust
use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let libtailscale_dir = manifest_dir.join("libtailscale");
    let archive = libtailscale_dir.join("libtailscale.a");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", libtailscale_dir.display());

    let status = Command::new("go")
        .args(["build", "-buildmode=c-archive", "-o"])
        .arg(&archive)
        .arg(".")
        .current_dir(&libtailscale_dir)
        .status()
        .expect("failed to run go build — is Go installed?");

    if !status.success() {
        panic!("go build failed with status: {}", status);
    }

    println!("cargo:rustc-link-search=native={}", libtailscale_dir.display());
    println!("cargo:rustc-link-lib=static=libtailscale");

    // On macOS, libtailscale needs these frameworks
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rustc-link-lib=framework=IOKit");
    }
    println!("cargo:rustc-link-lib=resolv");

    let bindings = bindgen::Builder::default()
        .header(libtailscale_dir.join("tailscale.h").to_str().unwrap())
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("unable to generate bindings");

    let out_path = manifest_dir.join("src/vendor/libtailscale.rs");
    bindings.write_to_file(&out_path).expect("couldn't write bindings");
}
```

- [ ] **Step 2: Remove cmake from Cargo.toml**

In `Cargo.toml`, delete the `cmake = "0.1.57"` line from `[build-dependencies]`.

- [ ] **Step 3: Verify build**

Run: `cargo build 2>&1`
Expected: Compiles and links successfully with no cmake errors.

- [ ] **Step 4: Commit**

```bash
git add build.rs Cargo.toml
git commit -m "fix: build system — absolute paths, explicit archive output, link directives, remove cmake"
```

---

### Task 2: Delete Dead Code & Fix Module Visibility (B9, B12)

**Files:**
- Delete: `src/glue/as_c_ptr.rs`
- Modify: `src/glue/mod.rs`
- Modify: `src/lib.rs`

**Context:** `IntoRawCPtr` is never used and leaks memory via `CString::into_raw()`. All public types are inaccessible because nothing is `pub`.

- [ ] **Step 1: Delete `src/glue/as_c_ptr.rs`**

Remove the file entirely.

- [ ] **Step 2: Update `src/glue/mod.rs`**

```rust
pub mod server;
```

(Remove the `mod as_c_ptr;` line, add `pub` to `mod server`)

- [ ] **Step 3: Update `src/lib.rs` with re-exports**

```rust
mod vendor;
pub mod glue;

pub use glue::server::{
    ConnectionHandler, FdControl, HandlerFactory, RawTsNetServer, TsNetError,
};
```

- [ ] **Step 4: Verify build**

Run: `cargo build 2>&1`
Expected: Compiles with no "unused" warnings for as_c_ptr.

- [ ] **Step 5: Commit**

```bash
git add -A src/glue/as_c_ptr.rs src/glue/mod.rs src/lib.rs
git commit -m "fix: remove dead IntoRawCPtr, make public API accessible"
```

---

### Task 3: Add Drop, Fix close() Error Handling, Add set_hostname (B3, B10, B8, B13)

**Files:**
- Modify: `src/glue/server.rs`

**Context:**
- B3: No `Drop` impl means leaked Go server handles.
- B10: `close()` calls `read_error()` after Go has already deleted the server from its map, so `tailscale_errmsg` returns EBADF.
- B8: `set_hostname` return value ignored in `new()`.
- B13: No standalone `set_hostname()` method.

- [ ] **Step 1: Write tests for Drop and set_hostname**

Add to the `#[cfg(test)] mod tests` block at the bottom of `server.rs`:

```rust
#[test]
fn set_hostname_method_exists() {
    // Type-check only — we can't call FFI in unit tests without the Go lib running
    fn _assert_set_hostname(server: &RawTsNetServer) {
        let _: Result<(), TsNetError> = server.set_hostname("test-host");
    }
}
```

- [ ] **Step 2: Add `set_hostname()` method**

Add after the `set_control_server` method (around line 211):

```rust
pub fn set_hostname(&self, hostname: &str) -> Result<(), TsNetError> {
    let hostname_c = str_to_c(hostname)?;
    unsafe {
        let err = libtailscale::tailscale_set_hostname(self.server, hostname_c.as_ptr());
        if err != 0 {
            return Err(self.read_error());
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Fix `new()` to check set_hostname return value**

Replace the `new` method:

```rust
pub fn new(hostname: &str) -> Result<Self, TsNetError> {
    unsafe {
        let server = libtailscale::tailscale_new();
        let s = RawTsNetServer { server };
        s.set_hostname(hostname)?;
        Ok(s)
    }
}
```

Note: This changes the return type from `Self` to `Result<Self, TsNetError>`. Update all call sites (the `dial_method_signature_compiles` test creates a server reference without calling `new`, so it won't break).

- [ ] **Step 4: Fix `close()` error handling (B10)**

Replace the `close` method:

```rust
pub fn close(&mut self) -> Result<(), TsNetError> {
    if self.server == -1 {
        return Ok(()); // already closed
    }
    // Read any pending error BEFORE close, since Go deletes the handle during close
    let err = unsafe { libtailscale::tailscale_close(self.server) };
    self.server = -1; // mark as closed regardless
    if err != 0 {
        // Can't call read_error() here — the Go side already deleted the server handle.
        // The Go implementation logs the error to its logger before returning -1.
        return Err(TsNetError::TAILSCALE("tailscale_close failed (see tailscale logs)".to_string()));
    }
    Ok(())
}
```

- [ ] **Step 5: Add `Drop` impl (B3)**

Add after the `impl RawTsNetServer` block:

```rust
impl Drop for RawTsNetServer {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test 2>&1`
Expected: All existing tests pass + new test passes. Some tests that call `new()` will need updating (see step 7).

- [ ] **Step 7: Fix any test compilation errors from `new()` return type change**

The `dial_method_signature_compiles` test references `&RawTsNetServer` without calling `new()`, so it should be fine. If any tests break, update them to use `new("x").unwrap()` or construct directly.

- [ ] **Step 8: Commit**

```bash
git add src/glue/server.rs
git commit -m "fix: add Drop impl, fix close() use-after-free, add set_hostname method"
```

---

### Task 4: Fix getremoteaddr Wrong Argument (B1)

**Files:**
- Modify: `src/glue/server.rs`

**Context:** `tailscale_getremoteaddr` takes `(listener_fd, conn_fd, buf, buflen)` but the code passes `(self.server, conn_fd, buf, buflen)`. The listener_fd is available in scope as `listener_fd`.

- [ ] **Step 1: Fix the tailscale_getremoteaddr call**

In the `listen` method, find (around line 351-353):

```rust
let remote_addr = unsafe {
    libtailscale::tailscale_getremoteaddr(self.server, conn_fd, buf.as_mut_ptr(), 255)
};
```

Replace with:

```rust
let remote_addr = unsafe {
    libtailscale::tailscale_getremoteaddr(listener_fd, conn_fd, buf.as_mut_ptr(), 255)
};
```

- [ ] **Step 2: Run tests**

Run: `cargo test 2>&1`
Expected: All tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/glue/server.rs
git commit -m "fix: pass listener_fd instead of server handle to tailscale_getremoteaddr"
```

---

### Task 5: Fix accept Error Handling (B2)

**Files:**
- Modify: `src/glue/server.rs`

**Context:** `tailscale_accept` returns -1 on error (with details in `tailscale_errmsg`) or `EBADF`. The current code reads `errno` via `io::Error::last_os_error()`, which gives wrong error messages. However, for the WouldBlock check specifically, this works because the underlying `recvmsg` sets errno. We need to handle both: WouldBlock from errno (no connection ready) and real errors from tailscale_errmsg.

- [ ] **Step 1: Fix accept error handling**

In the `listen` method, find the accept loop (around lines 338-348):

```rust
let res = unsafe {
    libtailscale::tailscale_accept(listener_fd, &mut conn_fd)
};
if res != 0 {
    let e = io::Error::last_os_error();
    if e.kind() != io::ErrorKind::WouldBlock {
        return Err(TsNetError::IO(e));
    }
    break;
}
```

Replace with:

```rust
let res = unsafe {
    libtailscale::tailscale_accept(listener_fd, &mut conn_fd)
};
if res != 0 {
    // EAGAIN/EWOULDBLOCK means no connection ready yet (from underlying recvmsg)
    let e = io::Error::last_os_error();
    if e.kind() == io::ErrorKind::WouldBlock {
        break;
    }
    // For real errors, use tailscale's error message
    return Err(self.read_error());
}
```

- [ ] **Step 2: Also fix the getremoteaddr error handling right below**

Find (around lines 354-359):

```rust
if remote_addr != 0 {
    let e = io::Error::last_os_error();
    if e.kind() != io::ErrorKind::WouldBlock {
        return Err(TsNetError::IO(e));
    }
    break;
}
```

Replace with:

```rust
if remote_addr != 0 {
    // Non-fatal: close conn and continue accepting
    unsafe { libc::close(conn_fd) };
    continue;
}
```

(Getting remote addr failure shouldn't abort the entire listener — just skip this connection.)

- [ ] **Step 3: Run tests**

Run: `cargo test 2>&1`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/glue/server.rs
git commit -m "fix: use tailscale_errmsg for accept errors, handle getremoteaddr failures gracefully"
```

---

### Task 6: Add Shutdown Mechanism to listen() (B4, B14)

**Files:**
- Modify: `src/glue/server.rs`

**Context:** `listen()` has an infinite loop with no exit. We need a shutdown signal. The cleanest approach: accept a shutdown fd (one end of a pipe) that the caller can close/write to trigger shutdown. This avoids adding `Arc<AtomicBool>` cross-thread complexity — the caller creates a pipe, passes one end, and drops/writes the other to signal stop.

- [ ] **Step 1: Add a `Listener` struct that holds the shutdown handle**

Add before the `RawTsNetServer` struct:

```rust
pub struct Listener {
    shutdown_fd: OwnedFd,
}

impl Listener {
    /// Signal the listener to shut down. This can be called from any thread.
    pub fn shutdown(&self) -> io::Result<()> {
        let n = unsafe {
            libc::write(
                self.shutdown_fd.as_raw_fd(),
                b"x".as_ptr() as *const libc::c_void,
                1,
            )
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
```

- [ ] **Step 2: Refactor listen() to return a Listener and accept shutdown**

Change the `listen` signature and add shutdown pipe logic:

```rust
pub fn listen<F: HandlerFactory>(
    &self,
    network: &str,
    addr: &str,
    factory: F,
) -> Result<Listener, TsNetError> {
    let network_c = str_to_c(network)?;
    let addr_c = str_to_c(addr)?;

    let mut listener_fd: c_int = 0;
    let err = unsafe {
        libtailscale::tailscale_listen(
            self.server,
            network_c.as_ptr(),
            addr_c.as_ptr(),
            &mut listener_fd,
        )
    };
    if err != 0 {
        return Err(self.read_error());
    }

    let _listener_owned = unsafe { OwnedFd::from_raw_fd(listener_fd) };
    unsafe { set_nonblocking(listener_fd) }?;

    // Create a pipe for shutdown signaling
    let mut pipe_fds = [0 as c_int; 2];
    if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
        return Err(TsNetError::IO(io::Error::last_os_error()));
    }
    let shutdown_read_fd = pipe_fds[0];
    let shutdown_write_fd = unsafe { OwnedFd::from_raw_fd(pipe_fds[1]) };
    let _shutdown_read_owned = unsafe { OwnedFd::from_raw_fd(shutdown_read_fd) };
    unsafe { set_nonblocking(shutdown_read_fd) }?;

    let mut poll = Poll::new()?;
    let mut events = Events::with_capacity(128);

    const LISTENER: Token = Token(0);
    const SHUTDOWN: Token = Token(usize::MAX);
    poll.registry()
        .register(&mut SourceFd(&listener_fd), LISTENER, Interest::READABLE)?;
    poll.registry()
        .register(&mut SourceFd(&shutdown_read_fd), SHUTDOWN, Interest::READABLE)?;

    let mut conns: HashMap<Token, ConnState<F::Handler>> = HashMap::new();
    let mut next_token: usize = 1;

    loop {
        poll.poll(&mut events, None)?;

        for event in &events {
            match event.token() {
                SHUTDOWN => {
                    // Clean up all connections
                    for (_, state) in conns.drain() {
                        let _ = poll.registry().deregister(&mut SourceFd(&state.fd));
                        unsafe { libc::close(state.fd) };
                    }
                    return Ok(Listener { shutdown_fd: shutdown_write_fd });
                }
                LISTENER => {
                    // ... (existing accept loop — see Task 4/5 for the fixed version)
                }
                token => {
                    // ... (existing connection handling — unchanged)
                }
            }
        }
    }
}
```

Wait — this is wrong. The `Listener` with the shutdown fd needs to be returned **before** the loop starts, not after shutdown. Let me redesign.

The correct pattern: `listen()` returns a `Listener` immediately. The event loop runs on a spawned thread. The `Listener` holds the shutdown write end.

**Revised Step 2:**

```rust
pub fn listen<F: HandlerFactory + Send + 'static>(
    &self,
    network: &str,
    addr: &str,
    factory: F,
) -> Result<Listener, TsNetError>
where
    F::Handler: Send,
{
    let network_c = str_to_c(network)?;
    let addr_c = str_to_c(addr)?;

    let mut listener_fd: c_int = 0;
    let err = unsafe {
        libtailscale::tailscale_listen(
            self.server,
            network_c.as_ptr(),
            addr_c.as_ptr(),
            &mut listener_fd,
        )
    };
    if err != 0 {
        return Err(self.read_error());
    }

    unsafe { set_nonblocking(listener_fd) }?;

    // Create a pipe for shutdown signaling
    let mut pipe_fds = [0 as c_int; 2];
    if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
        return Err(TsNetError::IO(io::Error::last_os_error()));
    }
    let shutdown_read_fd = pipe_fds[0];
    let shutdown_write_fd = unsafe { OwnedFd::from_raw_fd(pipe_fds[1]) };
    unsafe { set_nonblocking(shutdown_read_fd) }?;

    let server_handle = self.server;

    std::thread::spawn(move || {
        let _listener_owned = unsafe { OwnedFd::from_raw_fd(listener_fd) };
        let _shutdown_read_owned = unsafe { OwnedFd::from_raw_fd(shutdown_read_fd) };

        let mut poll = match Poll::new() {
            Ok(p) => p,
            Err(_) => return,
        };
        let mut events = Events::with_capacity(128);

        const LISTENER: Token = Token(0);
        const SHUTDOWN: Token = Token(usize::MAX);
        let _ = poll.registry()
            .register(&mut SourceFd(&listener_fd), LISTENER, Interest::READABLE);
        let _ = poll.registry()
            .register(&mut SourceFd(&shutdown_read_fd), SHUTDOWN, Interest::READABLE);

        let mut conns: HashMap<Token, ConnState<F::Handler>> = HashMap::new();
        let mut next_token: usize = 1;

        'outer: loop {
            if poll.poll(&mut events, None).is_err() {
                break;
            }

            for event in &events {
                match event.token() {
                    SHUTDOWN => {
                        for (_, state) in conns.drain() {
                            let _ = poll.registry().deregister(&mut SourceFd(&state.fd));
                            unsafe { libc::close(state.fd) };
                        }
                        break 'outer;
                    }
                    LISTENER => {
                        loop {
                            let mut conn_fd: c_int = 0;
                            let res = unsafe {
                                libtailscale::tailscale_accept(listener_fd, &mut conn_fd)
                            };
                            if res != 0 {
                                break; // no more pending connections
                            }

                            // Try to get remote addr (non-fatal if it fails)
                            let mut buf = vec![0 as c_char; 256];
                            let _ = unsafe {
                                libtailscale::tailscale_getremoteaddr(
                                    listener_fd, conn_fd, buf.as_mut_ptr(), 255,
                                )
                            };

                            if unsafe { set_nonblocking(conn_fd) }.is_err() {
                                unsafe { libc::close(conn_fd) };
                                continue;
                            }
                            while conns.contains_key(&Token(next_token)) || next_token == 0 || next_token == usize::MAX {
                                next_token = next_token.wrapping_add(1);
                            }
                            let token = Token(next_token);
                            next_token = next_token.wrapping_add(1);
                            if next_token == 0 || next_token == usize::MAX {
                                next_token = 1;
                            }

                            let conn_owned = unsafe { OwnedFd::from_raw_fd(conn_fd) };
                            if poll.registry()
                                .register(&mut SourceFd(&conn_fd), token, Interest::READABLE)
                                .is_err()
                            {
                                // OwnedFd will close on drop
                                continue;
                            }
                            let mut handler = factory.new_handler();
                            match handler.on_connect(conn_owned.as_raw_fd()) {
                                FdControl::KEEP => {
                                    conns.insert(token, ConnState {
                                        fd: conn_owned.into_raw_fd(),
                                        handler,
                                        write_buf: VecDeque::new(),
                                    });
                                }
                                FdControl::TAKE_OVER => {
                                    let fd = conn_owned.into_raw_fd();
                                    let _ = poll.registry().deregister(&mut SourceFd(&fd));
                                }
                            }
                        }
                    }
                    token => {
                        let should_close = if let Some(state) = conns.get_mut(&token) {
                            let mut close = false;

                            if event.is_readable() {
                                let mut buf = [0u8; 8192];
                                loop {
                                    let n = unsafe {
                                        libc::read(
                                            state.fd,
                                            buf.as_mut_ptr() as *mut libc::c_void,
                                            buf.len(),
                                        )
                                    };
                                    if n > 0 {
                                        state.handler.on_data(&buf[..n as usize]);
                                        if state.handler.is_done() {
                                            break;
                                        }
                                    } else if n == 0 {
                                        close = true;
                                        break;
                                    } else {
                                        let err = io::Error::last_os_error();
                                        if err.kind() != io::ErrorKind::WouldBlock {
                                            close = true;
                                        }
                                        break;
                                    }
                                }
                            }

                            if !close {
                                match try_flush(state) {
                                    Ok(wants_writable) => {
                                        let interest = if wants_writable {
                                            Interest::READABLE | Interest::WRITABLE
                                        } else {
                                            Interest::READABLE
                                        };
                                        let _ = poll.registry().reregister(
                                            &mut SourceFd(&state.fd),
                                            token,
                                            interest,
                                        );
                                    }
                                    Err(_) => { close = true; }
                                }
                            }

                            if !close && state.handler.is_done() {
                                close = true;
                            }

                            close
                        } else {
                            false
                        };

                        if should_close {
                            if let Some(state) = conns.remove(&token) {
                                let _ = poll.registry().deregister(&mut SourceFd(&state.fd));
                                unsafe { libc::close(state.fd) };
                            }
                        }
                    }
                }
            }
        }
    });

    Ok(Listener { shutdown_fd: shutdown_write_fd })
}
```

- [ ] **Step 3: Update the Listener re-export in lib.rs**

Add `Listener` to the `pub use` in `src/lib.rs`:

```rust
pub use glue::server::{
    ConnectionHandler, FdControl, HandlerFactory, Listener, RawTsNetServer, TsNetError,
};
```

- [ ] **Step 4: Run tests**

Run: `cargo test 2>&1`
Expected: All tests pass. The `dial_method_signature_compiles` test doesn't call `listen()` so it's unaffected.

- [ ] **Step 5: Commit**

```bash
git add src/glue/server.rs src/lib.rs
git commit -m "feat: listen() returns Listener with shutdown(), runs event loop on background thread"
```

---

### Task 7: Add getips() Wrapper (B15)

**Files:**
- Modify: `src/glue/server.rs`

**Context:** The C API has `tailscale_getips(sd, buf, buflen)` which returns comma-separated IPs. No Rust wrapper exists.

- [ ] **Step 1: Write a type-check test**

```rust
#[test]
fn getips_method_signature_compiles() {
    fn _assert_getips(server: &RawTsNetServer) {
        let _: Result<String, TsNetError> = server.getips();
    }
}
```

- [ ] **Step 2: Add getips() method**

Add to `impl RawTsNetServer`:

```rust
pub fn getips(&self) -> Result<String, TsNetError> {
    let mut buf = vec![0 as c_char; 256];
    unsafe {
        let err = libtailscale::tailscale_getips(self.server, buf.as_mut_ptr(), buf.len());
        if err != 0 {
            return Err(self.read_error());
        }
        let ips = CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
        Ok(ips)
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test 2>&1`
Expected: All tests pass including new signature test.

- [ ] **Step 4: Commit**

```bash
git add src/glue/server.rs
git commit -m "feat: add getips() wrapper for tailscale_getips"
```

---

### Task 8: Fix Token Collision with SHUTDOWN in listen's next_token (B7 adjacent)

**Files:**
- Modify: `src/glue/server.rs`

**Context:** The `next_token` counter could theoretically wrap around to `usize::MAX` (the SHUTDOWN token) or 0 (the LISTENER token). The revised listen loop in Task 6 already handles this with the `while` check, but we should also make sure the original `drive_conn` function (used by `dial`) properly owns its fd.

- [ ] **Step 1: Verify drive_conn fd ownership is sound**

In `drive_conn`, the fd is wrapped in `OwnedFd` at the top (line 106) for auto-close, then used as a raw fd throughout. This is correct — `_owned` keeps the fd alive and closes it on return. No change needed here.

- [ ] **Step 2: Commit (skip if no changes)**

No commit needed — this was a verification step.

---

### Task 9: Final Verification & Cleanup

**Files:**
- All modified files

- [ ] **Step 1: Run full test suite**

Run: `cargo test 2>&1`
Expected: All tests pass.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy 2>&1`
Expected: No errors (warnings from generated bindings are OK).

- [ ] **Step 3: Verify public API is accessible**

Create a quick compile check:

Run: `cargo doc --no-deps 2>&1`
Expected: Documentation generates showing `RawTsNetServer`, `ConnectionHandler`, `HandlerFactory`, `Listener`, `TsNetError`, `FdControl` as public items.

- [ ] **Step 4: Final commit if any cleanup needed**

```bash
git add -A
git commit -m "chore: final cleanup for production readiness"
```
