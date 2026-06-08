# Acknowledgments

AgentArk stands on the shoulders of a large open-source ecosystem. None of this would exist without the maintainers of these projects:

**Core runtime**

| Project                                                                                                               | Used for                                                         |
| :-------------------------------------------------------------------------------------------------------------------- | :--------------------------------------------------------------- |
| [Rust](https://www.rust-lang.org/)                                                                                    | Core language - memory safety, performance, fearless concurrency |
| [Tokio](https://tokio.rs/)                                                                                            | Async runtime powering all concurrent operations                 |
| [Hyper](https://hyper.rs/) + [Axum](https://github.com/tokio-rs/axum) + [Tower](https://github.com/tower-rs/tower)    | HTTP stack, server, router, middleware                           |
| [Tokio-Tungstenite](https://github.com/snapview/tokio-tungstenite)                                                    | WebSocket implementation (channels + companion devices)          |
| [Reqwest](https://github.com/seanmonstar/reqwest)                                                                     | HTTP client for every outbound API call                          |
| [Futures](https://github.com/rust-lang/futures-rs)                                                                    | Async streams, combinators, and background pipeline plumbing      |
| [Serde](https://serde.rs/) + [serde_json](https://github.com/serde-rs/json) + [TOML](https://github.com/toml-rs/toml) | Serialization across the entire codebase                         |
| [Tracing](https://github.com/tokio-rs/tracing)                                                                        | Structured logging and diagnostics                               |
| [Clap](https://github.com/clap-rs/clap)                                                                               | CLI argument parsing                                             |
| [Anyhow](https://github.com/dtolnay/anyhow) + [Thiserror](https://github.com/dtolnay/thiserror)                       | Error handling                                                   |

**Storage, data, and scheduling**

| Project                                                                                                              | Used for                                         |
| :------------------------------------------------------------------------------------------------------------------- | :----------------------------------------------- |
| [PostgreSQL](https://www.postgresql.org/) + [pgvector](https://github.com/pgvector/pgvector)                         | Primary database and vector embedding store      |
| [SeaORM](https://www.sea-ql.org/SeaORM/) + [SQLx](https://github.com/launchbadge/sqlx)                               | Database ORM and query layer                     |
| [Chrono](https://github.com/chronotope/chrono) + [chrono-tz](https://github.com/chronotope/chrono-tz)                | Time and timezone handling                       |
| [cron](https://github.com/zslayton/cron)                                                                              | Cron expression parsing for scheduled tasks      |
| [FastEmbed](https://github.com/Anush008/fastembed-rs)                                                                | Local embedding generation for memory and search |
| [pdf-extract](https://github.com/jrmuizel/pdf-extract)                                                               | PDF text extraction for documents                |
| [notify](https://github.com/notify-rs/notify) + [walkdir](https://github.com/BurntSushi/walkdir)                     | Filesystem watching and safe directory scans     |
| [zip](https://github.com/zip-rs/zip2)                                                                                 | Extension-pack, DOCX, and archive handling       |

**Security and cryptography**

| Project                                                                                                                                          | Used for                                       |
| :----------------------------------------------------------------------------------------------------------------------------------------------- | :--------------------------------------------- |
| [Ring](https://github.com/briansmith/ring) + [Rustls](https://github.com/rustls/rustls)                                                          | TLS and general-purpose cryptography           |
| [ed25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek) + [x25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek) | Device pairing and signed identities           |
| [AES-GCM](https://github.com/RustCrypto/AEADs) + [Argon2](https://github.com/RustCrypto/password-hashes)                                         | Secret-at-rest encryption and password hashing |
| [SHA-2](https://github.com/RustCrypto/hashes) + [BLAKE3](https://github.com/BLAKE3-team/BLAKE3)                                                  | Hashing for integrity and content addressing   |
| [Zeroize](https://github.com/iqlusioninc/crates/tree/main/zeroize)                                                                               | Wiping secrets from memory                     |

**Execution, automation, and integrations**

| Project                                                                               | Used for                                            |
| :------------------------------------------------------------------------------------ | :-------------------------------------------------- |
| [Wasmtime](https://wasmtime.dev/)                                                     | WebAssembly sandbox for safe code execution         |
| [Docker](https://www.docker.com/) + [Bollard](https://github.com/fussybeaver/bollard) | Container runtime and Rust Docker client            |
| [Playwright](https://playwright.dev/)                                                 | Interactive browser automation and operator handoff |
| [Lightpanda](https://github.com/lightpanda-io/browser)                                | Fast headless browser for content extraction        |
| [scraper](https://github.com/rust-scraper/scraper)                                   | HTML parsing for search, app inspection, and guards |
| [russh](https://github.com/warp-tech/russh)                                           | SSH client for remote-action flows                  |
| [SearXNG](https://github.com/searxng/searxng)                                         | Self-hosted metasearch (optional research backend)  |
| [Model Context Protocol](https://modelcontextprotocol.io/)                            | Open standard for pluggable tool surfaces           |
| [Cloudflared](https://github.com/cloudflare/cloudflared)                              | Public-link remote access via Cloudflare Tunnel     |
| [Tailscale](https://tailscale.com/)                                                   | Private tailnet access with end-to-end encryption   |

**Messaging channels and bots**

| Project                                                                                                                       | Used for                   |
| :---------------------------------------------------------------------------------------------------------------------------- | :------------------------- |
| [Teloxide](https://github.com/teloxide/teloxide)                                                                              | Telegram bot framework     |
| [Baileys](https://github.com/WhiskeySockets/Baileys)                                                                          | Embedded WhatsApp bridge   |
| Platform HTTP/WebSocket APIs (Slack, Discord, Matrix, Signal, iMessage, Google Chat, Teams, LINE, WeChat, QQ, and others)     | Messaging-channel adapters |
| [Lettre](https://github.com/lettre/lettre)                                                                                    | SMTP email delivery        |

**Frontend and visualization**

| Project                                                                                                             | Used for                                  |
| :------------------------------------------------------------------------------------------------------------------ | :---------------------------------------- |
| [React](https://react.dev/) + [TypeScript](https://www.typescriptlang.org/)                                         | Web UI foundation                         |
| [Vite](https://vitejs.dev/)                                                                                         | Frontend build and dev server             |
| [MUI](https://mui.com/) + [Emotion](https://emotion.sh/)                                                            | Component library and CSS-in-JS           |
| [TanStack Query](https://tanstack.com/query)                                                                        | Server-state management and data fetching |
| [Zustand](https://github.com/pmndrs/zustand)                                                                        | Client-state management                   |
| [ECharts](https://echarts.apache.org/) + [echarts-for-react](https://github.com/hustcc/echarts-for-react)           | Charts and visualizations                 |
| [react-markdown](https://github.com/remarkjs/react-markdown) + [remark-gfm](https://github.com/remarkjs/remark-gfm) | Markdown rendering in chat                |
| [Lucide](https://lucide.dev/)                                                                                       | Icon set                                  |

**Native GUI (optional feature)**

| Project                                                                | Used for                  |
| :--------------------------------------------------------------------- | :------------------------ |
| [egui](https://www.egui.rs/) + [eframe](https://github.com/emilk/egui) | Native desktop GUI option |

If we missed a project that's load-bearing for AgentArk, please open a PR - we want to thank everyone.
