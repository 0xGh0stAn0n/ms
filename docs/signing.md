# Code signing

A certificate's key lives in hardware, so it is never a CI secret. Two ways:

## SignPath (CI signs on a version tag)

1. In SignPath, create a project `maplesyrup` linked to this repository (GitHub trusted build system), and a signing policy `release-signing` that uses the certificate.
2. Add two artifact configurations:
   - `exe`: a zip with `maplesyrup.exe` and `vision_debug.exe`, each `<pe-file>` with `<authenticode-sign/>`.
   - `installer`: a zip with one `<pe-file path="MapleSyrup-Setup-*.exe">` with `<authenticode-sign/>`.
3. In the repository, set the secret `SIGNPATH_API_TOKEN` and the variable `SIGNPATH_ORGANIZATION_ID`. Set `SIGNPATH_PROJECT` and `SIGNPATH_POLICY` only if the names differ from the ones above.

From then on, `standalone.yml` does this on a version tag, or on a run with "publish" ticked:
- signs the programs before they are packed;
- signs the installer;
- checks both signatures, and stops the release if either is invalid.

The zip, the installer and the manifest all carry the signed files, so the updater's hashes match.

Every other build signs a copy with a throwaway test certificate and checks it (`logs/signing.log`), so the steps are known to work. That copy is never shipped.

## Certum (signed by hand)

The manifest holds each file's SHA-256, so signing has to come before the manifest:

1. Build on the tag with SignPath unset, and download `MapleSyrup-<v>.exe` from the draft release.
2. Sign it and check the signature:
   ```
   signtool sign /n "<certificate subject>" /fd sha256 /tr http://time.certum.pl /td sha256 MapleSyrup-<v>.exe
   signtool verify /pa MapleSyrup-<v>.exe
   ```
3. Build the installer from the signed exe (`iscc` as in `standalone.yml`), sign and verify it the same way.
4. Remake `manifest.json` and its signature from the signed files, then upload them in place of the unsigned ones.

That is four manual steps for each release, so SignPath is the one to use.
