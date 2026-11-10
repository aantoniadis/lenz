# Prepare, validate, package and publish 0.0.14
Status: claimed
Type: task

Bump workspace/lock metadata to0.0.14, write release notes covering PR49–52,
run required automated gates and full release workloads, then tag the validated
commit. Monitor all three native package jobs, verify exact asset identities and
CLI/bundle checks, publish the existing draft as a beta and update real checksum
pins in the repository and Homebrew tap. Preserve failed attempts and limitations.
