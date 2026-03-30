# WebdavBridge

WebDAV proxy with metadata caching, rate limiting, and symlink support for video/audio streaming.

## Overview

WebdavBridge acts as a proxy between downstream WebDAV clients and an upstream WebDAV server. It provides:

- **Metadata caching** via sled embedded database for fast PROPFIND responses
- **Content caching** via local filesystem for repeated access
- **Rate limiting** to prevent overloading the upstream server
- **Range request support** (single and multi-range) for media streaming
- **Symlink functionality** allowing downstream clients to create, move, and override files without affecting upstream

## Architecture

```
┌─────────────────────────────────────────┐
│     Downstream WebDAV Clients           │
│ GET/HEAD/PROPFIND/COPY/MOVE/PUT/DELETE  │
└──────────────┬──────────────────────────┘
               │
         WebdavBridge Server
               │
      ┌────────┴────────────────────┐
      │                             │
  MetadataCache              ContentFetchTask
  (sled database)            (single-threaded)
      │                             │
      │                        UpstreamClient
      │                       (rate-limited)
      │                             │
      └──────────┬──────────────────┘
                 │
         Upstream WebDAV Server
```

## Symlink Functionality

WebdavBridge supports a symlink-like mechanism that enables downstream read/write separation from the upstream server:

### How It Works

- **COPY**: Creates a symlink at the destination pointing to the same upstream target as the source. Does not access or copy actual content from upstream.
- **MOVE**: Moves a symlink to a new path. The upstream target reference remains unchanged.
- **GET**: If the path is a symlink with a local override, reads from the local content cache. If no local override, fetches from upstream (through the rate limiter).
- **PUT**: Writes content to the local content cache and marks the symlink as having a local override. The upstream file is never modified.
- **DELETE**: Deletes the symlink metadata and any local override content. The upstream file is never affected.

### Key Properties

- Symlinks only exist in the downstream MetadataCache; the upstream server is never aware of them
- Writing to a symlink creates a local override — subsequent reads return the local version
- If the upstream target is deleted, accessing the symlink returns 404 and the symlink is automatically cleaned up
- Directory symlinks are supported with recursive mapping of children
- Cycle detection prevents circular symlink chains (configurable max depth)

### Restrictions

- Only symlink resources can be moved, written to, or deleted via downstream operations
- Non-symlink (upstream) resources return `403 Forbidden` for MOVE/PUT/DELETE
- Symlink chain depth is limited (default: 3) to prevent infinite loops

## Configuration

Configuration is loaded from a YAML file (default: `config.yaml`):

```yaml
upstream_url: "http://upstream-server:8081"
upstream_username: null
upstream_password: null
cache_dir: "./cache"
metadata_db_path: "./metadata.db"
rate_limit_permits: 2
metadata_update_interval_secs: 300
max_depth: 10
server_bind: "127.0.0.1:8080"
server_prefix: "/"
max_symlink_depth: 3
```

### Configuration Options

| Option | Default | Description |
|--------|---------|-------------|
| `upstream_url` | `http://localhost:8081` | Upstream WebDAV server URL |
| `upstream_username` | `null` | Upstream auth username (optional) |
| `upstream_password` | `null` | Upstream auth password (optional) |
| `cache_dir` | `./cache` | Local content cache directory |
| `metadata_db_path` | `./metadata.db` | sled metadata database path |
| `rate_limit_permits` | `2` | Max concurrent upstream connections |
| `metadata_update_interval_secs` | `300` | Background metadata sync interval (seconds) |
| `max_depth` | `10` | Max PROPFIND recursion depth |
| `server_bind` | `127.0.0.1:8080` | Server listen address |
| `server_prefix` | `/` | URL prefix for the WebDAV server |
| `max_symlink_depth` | `3` | Maximum symlink chain resolution depth |

## Usage

### Running

```bash
# With default config file (config.yaml)
cargo run

# With custom config path
WEBDAV_BRIDGE_CONFIG=/path/to/config.yaml cargo run
```

### Supported WebDAV Methods

| Method | Description |
|--------|-------------|
| `GET` | Retrieve file content (supports Range headers) |
| `HEAD` | Get file metadata |
| `PROPFIND` | List directory contents |
| `COPY` | Create symlink at destination |
| `MOVE` | Move symlink to new path |
| `PUT` | Write local override to symlink |
| `DELETE` | Delete symlink and local override |

### Examples

```bash
# List directory
curl -X PROPFIND http://localhost:8080/

# Get a file
curl http://localhost:8080/movies/video.mp4 -o video.mp4

# Get a range
curl -H "Range: bytes=0-1023" http://localhost:8080/movies/video.mp4

# Create a symlink via COPY
curl -X COPY \
  -H "Destination: http://localhost:8080/local/my-video.mp4" \
  http://localhost:8080/movies/video.mp4

# Move a symlink
curl -X MOVE \
  -H "Destination: http://localhost:8080/local/renamed.mp4" \
  http://localhost:8080/local/my-video.mp4

# Write a local override
curl -X PUT --data-binary @new-content.mp4 \
  http://localhost:8080/local/renamed.mp4

# Delete a symlink
curl -X DELETE http://localhost:8080/local/renamed.mp4
```

## Development

### Building

```bash
cargo build
```

### Testing

```bash
cargo test
```

### Project Structure

```
src/
├── main.rs                  # Binary entry point and HTTP routing
├── lib.rs                   # Library root
├── config.rs                # Configuration management
├── webdav/
│   ├── types.rs             # Core types (WebdavResource, RangeSpec, errors)
│   ├── server.rs            # WebdavServer (downstream request handlers)
│   ├── client.rs            # UpstreamClient (upstream WebDAV client)
│   └── dav_fs.rs            # DavFileSystem trait implementation
├── cache/
│   ├── metadata.rs          # MetadataCache (sled database)
│   └── content.rs           # ContentCache (filesystem)
├── tasks/
│   ├── metadata_update.rs   # Background metadata sync task
│   └── content_fetch.rs     # Content fetch task
└── resume/
    ├── mod.rs               # RateLimiter
    └── range.rs             # Range header parsing & formatting
```

## License

MIT