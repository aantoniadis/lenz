# Publish Lenz 0.0.14 beta
Status: claimed

Publish the merged main tree after PR52 as the next beta, following the existing
three-platform tag packaging and verified-draft publication procedure. Validate
the version commit with normal Cargo profiles, local CI, performance gates and
the full suite. Verify platform archives, checksum sidecars, packaged CLI routing
and macOS bundle signatures, then publish exact assets and update Homebrew/AUR
metadata and the existing Homebrew tap. AUR registration remains unavailable.
Keep raw evidence under perf-results/release-0.0.14/. Report physical/native
manual coverage as unverified where unavailable; never mark it passed.
