# End-to-End Tests

Comprehensive E2E tests for the Simple Photos sync engine and all features.

## Prerequisites

```bash
pip3 install pytest requests
```

## Running

### Quick: single-server tests (no backup)
```bash
# Start the primary server on port 8090 with test config
cd server && cargo run & 
# Then run:
pytest tests/ -v -k "not backup_sync"
```

### Full: multi-server tests (primary + backup)
```bash
# The test harness automatically starts servers on ephemeral ports.
# Just run from the repo root:
pytest tests/ -v

# Or run specific test modules:
pytest tests/test_01_auth.py -v
pytest tests/test_02_photos.py -v
pytest tests/test_03_blobs.py -v
pytest tests/test_04_trash.py -v
pytest tests/test_05_albums_shared.py -v
pytest tests/test_06_albums_secure.py -v
pytest tests/test_07_tags_search.py -v
pytest tests/test_08_edit_copies.py -v
pytest tests/test_09_sync_engine.py -v
pytest tests/test_10_backup_restore.py -v
pytest tests/test_11_multi_user.py -v
pytest tests/test_12_edge_cases.py -v
```

### Using external servers (skip auto-start)
```bash
export E2E_PRIMARY_URL=http://localhost:8080
export E2E_BACKUP_URL=http://localhost:8081
export E2E_BACKUP_API_KEY=your-key
pytest tests/ -v
```

## Architecture

- `conftest.py` — Fixtures that start/stop server instances, create test users, provide API clients
- `helpers.py` — Shared HTTP helpers, assertion utilities, test data generators
- `test_01_*` through `test_12_*` — Ordered test modules covering every feature
