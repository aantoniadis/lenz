# Shared archive snapshots

`lenz-archive` browses and changes archive bytes in memory. It never extracts paths
onto disk or writes an outer file. The caller owns atomic disk replacement,
external-change detection, cancellation state, nested aggregate limits, and bounded
snapshot caching. `Index::resident_bytes()` includes unpacked TAR storage for that
cache budget.

`Kind::of` retains filename discovery semantics. `Kind::detect(name, bytes)` is for
an already selected archive layer: bounded content signatures retain its format
when canonicalizing a symlink changes the filename suffix. It falls back to the
expected suffix for useful malformed-format errors; it does not validate or extract.

`Index::open` owns one shared `Arc<[u8]>` snapshot. `MemberId` identifies an indexed
entry in that revision; reopen and resolve a unique normalized path when moving to
a newer revision. `stored_path` retains original header bytes, while `path` is a
normalized virtual location. GNU/PAX effective names are reflected in `path`.
Directories, links, metadata records and unsupported special entries are distinct
member kinds. Link reads return target bytes; only supported regular files can be
replaced. Callers should hide metadata records from ordinary tree listings.

ZIP lists central-directory metadata without decompressing members. Reads decode
one requested payload and check its size and CRC. Replacing several members copies
untouched compressed payloads with `ZipWriter::raw_copy_file`; it does not decode
them. Before returning a candidate, it checks entry identities, exact raw untouched
compressed bytes, archive/member comments, stored names, timestamps, permissions,
and local/central extras. ZIP64 size/offset fields are rebuilt for the new layout;
the archive framing need not be byte-identical. A writer that cannot preserve an
entry's metadata returns an explicit error and no candidate.

The locked ZIP reader resolves duplicate stored names to the last entry. Such
containers are explicitly read-only. Normalized aliases are individually read-only.
Encrypted containers, executable prefixes, signatures, trailing records, and
unsupported ZIP64 extensible metadata also have explicit preserving-write limits.
Opaque extra-field padding remains browsable but has an explicit write limit.
Unknown compression methods remain listable; only methods supported for writing
are editable.

TAR indexes a bounded unpacked stream without hashing payloads. Untouched raw
records, extension blocks, links, padding, and trailing bytes survive exactly.
Changed ordinary records retain their header except size/checksum and contain the
new payload and padding. PAX size overrides and sparse/special files are read-only.
GNU long names/links and ordinary PAX metadata remain attached to their entries.

Plain TAR, gzip, bzip2, XZ and Zstd retain their format. Compressed TAR rewrites its
wrapper and retains exact uncompressed records; compression settings may change.
Gzip retains its original complete header, including extra/name/comment/header-CRC
bytes. Multi-stream gzip and Zstd skippable metadata are browsable and explicitly
read-only because a new wrapper would discard their framing metadata.

Defaults are 512 MiB stored bytes, 2 GiB unpacked bytes, 1 GiB per opened member,
and 100,000 raw entries. ZIP applies the unpacked budget to declared aggregate
member sizes before opening payloads. TAR also applies it to retained index metadata
so repeated global PAX names cannot amplify a small stream without a bound.
XZ/LZMA/Zstd decoder dictionaries are bounded
at 128 MiB. Candidate output allocation is capped even when a codec requires an
infallible output sink. Nested callers must additionally bound depth and aggregate
work across layers.

The independent fixtures in `tests/fixtures` come from Python's standard archive
codecs and system libzstd. `generate.py` documents their construction. The optional
`testing` feature provides small in-memory ZIP/TAR helpers for other crates' tests.
