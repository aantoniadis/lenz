---
status: draft
tags:
  - service
  - accounting
updated: 2026-10-08
---
# Ledger

A small double-entry bookkeeping service with a [[JSON API]]. Read the [ledger guide](https://example.com/guide) for ==account rules==.[^rules]

> [!note]
> Balances are recomputed nightly.

> [!warning] Size limit
> Requests larger than `2 MiB` are refused.

> [!tip]- Folded tip
> Use `--dry-run` to preview postings.

## Running

Start the server with the default settings:

```sh
cargo run --release -- --config ledger.toml
```

### Checklist

- [x] Create the database
- [ ] Load the chart of accounts
- [ ] Import opening balances

### Configuration

- `listen` — address to bind
  - IPv4 or IPv6
  - port required
- `workers` — number of worker threads

| Key | Default | Meaning |
| --- | --- | --- |
| `listen` | `0.0.0.0:8080` | address to bind |
| `workers` | CPU count | worker threads |


![Ledger diagram](diagram.png)

---

#### Notes

See also [[Accounts]] and #howto.

[^rules]: Rules are listed in `docs/rules.md`.
