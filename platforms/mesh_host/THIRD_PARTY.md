# Third-party material

Apache-2.0 applies to original Mesh Lab work. It does not replace the licenses
of dependencies, tools, or external conformance vectors.

## Material included in the repository

| Material | Provenance and terms |
|---|---|
| Gradle wrapper (`app/android/gradlew`, `gradlew.bat`, and `gradle/wrapper/gradle-wrapper.jar`) | Gradle, under [Apache-2.0](https://github.com/gradle/gradle/blob/master/LICENSE). A copy of Apache-2.0 is retained at the root of this distribution. |
| Noise XX vector (`vectors/session/noise-xx-cacophony.json`) | Extracted unchanged from the [Snow 0.10.0 vectors](https://docs.rs/crate/snow/0.10.0/source/tests/vectors/cacophony.txt); Snow declares Apache-2.0 OR MIT. The original collection is [Cacophony](https://github.com/haskell-cryptography/cacophony), whose [LICENSE](https://github.com/haskell-cryptography/cacophony/blob/master/LICENSE) is Unlicense. See the vector directory README as well. |
| HPKE vector (`vectors/crypto/hpke-rfc9180.json`) | Comes from the [CFRG HPKE project vectors](https://github.com/cfrg/draft-irtf-cfrg-hpke/blob/master/test-vectors.json). Its [contribution terms](https://github.com/cfrg/draft-irtf-cfrg-hpke/blob/master/CONTRIBUTING.md) refer to BCP 78/79 and IETF Trust provisions, including those covering code components. No original authorship of that data is claimed. |

The key values in these vectors are public test fixtures, not installation
credentials or production material.

## Dependencies resolved during builds

Versions are pinned in `Cargo.lock` and in each package's `pubspec.lock`.
Rust, Flutter/Dart, and native components retain their respective owners'
notices. SQLCipher and its cryptographic provider, among others, are included
during native builds and are not relicensed as original Mesh Lab work.

This document identifies provenance; it is not an exhaustive license inventory
for a compiled binary. Anyone distributing APKs, IPAs, frameworks, or other
binaries must include the texts and notices of the dependencies actually
included in that artifact.
