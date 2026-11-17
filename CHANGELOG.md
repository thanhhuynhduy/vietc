# Changelog

## Unreleased

### Security
- Harden installation and runtime security (04dc234)
- Replace world-readable/writable input-device rules with active-session access. Upgrade revokes legacy world permissions; uninstall attempts extended ACL cleanup and reapplies distribution rules. **Device policy changes take effect only when the updated installer/uninstaller is run.**
- Harden daemon input handling (f2f1a58)

### Fixed
- Fix clipboard handling and add release evidence documentation (7c247d2)
- Fix daemon input handling hardening (f2f1a58)
- Fix Vietnamese engine behavior refinements (737c0d7)
- Fix Telex mark backtracking across vowels (c67546a)
- Fix errors after audit (c7b1258)
- Fix installation on Fedora 44 Cosmic DE (67d9180)
- Fix live text typing test issues (26fda23)

### Added
- Release and supply-chain automation (f0f539f)
- Allow free diacritic placement (008e6ed, dacbbb0)
- Split protocol input and clipboard modules (0526c0d)

### Refactored
- Refactor code after live test of Vietnamese text (26fda23)

### Chore
- Stop tracking runtime artifacts (08a2f82)
- Patch vulnerable web dependencies (7779740)
- Remove deploy GitHub workflow (2eea28e)

### Documentation
- Document clipboard fix and release evidence (7c247d2)
- Update README (aa28143)
