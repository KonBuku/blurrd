# Security policy

## Supported versions

blurrd has not been released yet. Once it has, only the latest release
receives security fixes.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's private
vulnerability reporting:
<https://github.com/KonBuku/blurrd/security/advisories/new>

Do not open public issues, discussions or pull requests for security
problems. You will get a first response within 7 days. Please include the
affected version, the steps to reproduce and, if possible, a minimal input
file that triggers the problem. Do not send recordings in which other
players' names are readable.

## Scope

blurrd processes local video files offline and does not connect to the
network. Relevant issues include:

- crafted media files that cause memory corruption, crashes or excessive
  resource use while probing, decoding or encoding;
- bugs in the unsafe code and FFI bindings (FFmpeg, ONNX Runtime, Windows
  APIs);
- problems that let the desktop app read or write files the user did not
  choose;
- integrity problems with release artefacts or the installer.

A player name that is not blurred is a normal bug, not a security issue.
Please open a regular issue with the timestamp, the HUD region and the
resolution, never with the unblurred footage.
