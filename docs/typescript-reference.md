# Rust Reference for TypeScript Developers

## Toolchain

| TypeScript | Rust |
|---|---|
| `npm install` | `cargo add <crate>` |
| `npm run build` | `cargo build` |
| `npm start` | `cargo run` |
| `npm test` | `cargo test` |
| `npx tsc --noEmit` | `cargo check` |
| `prettier` | `cargo fmt` |
| `eslint` | `cargo clippy` |
| `package.json` | `Cargo.toml` |
| `package-lock.json` | `Cargo.lock` |
| `node_modules/` | `~/.cargo/registry/` (global) |
| `npm create vite@latest` | `cargo new my-project` |
| `npmjs.com` | `crates.io` |
| `docs.npmjs.com` | `docs.rs` |

## Types

| TypeScript | Rust |
|---|---|
| `string` | `String` (owned) / `&str` (borrowed) |
| `number` | `i32`, `u32`, `i64`, `f64`, … (be explicit) |
| `boolean` | `bool` |
| `T \| null \| undefined` | `Option<T>` → `Some(v)` / `None` |
| `T \| Error` (throws) | `Result<T, E>` → `Ok(v)` / `Err(e)` |
| `any` | ❌ doesn't exist — use generics |
| `T[]` / `Array<T>` | `Vec<T>` |
| `[T, U]` tuple | `(T, U)` |
| `Record<K, V>` / `Map<K,V>` | `HashMap<K, V>` |
| `interface` / `type` | `struct` |
| `enum` | `enum` (much more powerful — can hold data) |
| `(a: A) => B` | `fn(A) -> B` / `impl Fn(A) -> B` |

## Common Patterns

```rust
// Optional chaining: foo?.bar → foo.map(|f| f.bar)
let len = name.map(|n| n.len());

// Nullish coalescing: foo ?? "default" → foo.unwrap_or("default")
let val = maybe_val.unwrap_or("default");

// Error propagation: throw/try-catch → ? operator (inside fn returning Result)
let content = std::fs::read_to_string("file.txt")?;

// Destructuring (similar)
let (x, y) = (1, 2);
let Point { x, y } = point;

// Closures (similar, but ownership matters)
let add = |a, b| a + b;
vec.iter().map(|x| x * 2).collect::<Vec<_>>();

// Async/await (needs a runtime — we use tokio)
async fn fetch() -> Result<String> { ... }
let result = fetch().await?;
```

## Structs vs Classes

```typescript
// TypeScript
class User {
  constructor(public name: string, public age: number) {}
  greet() { return `Hi, ${this.name}`; }
}
```

```rust
// Rust — no classes, use struct + impl
struct User { name: String, age: u32 }

impl User {
    fn new(name: &str, age: u32) -> Self {
        User { name: name.to_string(), age }
    }
    fn greet(&self) -> String {
        format!("Hi, {}", self.name)
    }
}
```

## Enums (like TS discriminated unions)

```rust
// Rust enums can hold data per variant
enum Shape {
    Circle(f64),
    Rect { w: f64, h: f64 },
}

let area = match shape {
    Shape::Circle(r) => std::f64::consts::PI * r * r,
    Shape::Rect { w, h } => w * h,
};
```

## Ownership in 30 Seconds

- Every value has **one owner**. When the owner goes out of scope, it's dropped (no GC).
- **Move**: `let b = a;` — `a` is no longer valid (unlike TS where both point to the same object).
- **Borrow** `&T`: read-only reference, many allowed at once.
- **Borrow** `&mut T`: mutable reference, only **one** at a time, no other borrows active.
- `clone()` is the escape hatch (explicit deep copy).

## Cargo.toml

```toml
[package]
name = "my-app"
version = "0.1.0"
edition = "2024"

[dependencies]
anyhow = "1"                                        # error handling
clap = { version = "4", features = ["derive"] }     # CLI args
tokio = { version = "1", features = ["full"] }      # async runtime
serde = { version = "1", features = ["derive"] }    # serialization

[dev-dependencies]
# test-only deps (like devDependencies)
```

## Useful Crates (npm equivalents)

| Purpose | Crate |
|---|---|
| Error handling | `anyhow`, `thiserror` |
| Serialization | `serde` + `serde_json` |
| Async runtime | `tokio` |
| HTTP client | `reqwest` |
| CLI parsing | `clap` |
| Logging | `tracing` |
| Regex | `regex` |
| Dates | `chrono` |
| Env vars | `dotenvy` |
| UUIDs | `uuid` |
