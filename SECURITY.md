# Security policy

## Reporting a vulnerability

Please report a suspected vulnerability in TigerSetup — the `tiger-setup`
builder, the installer engine, the loader, or an installer TigerSetup builds —
privately, through GitHub's private vulnerability reporting:
<https://github.com/rkozlowski/TigerSetup/security/advisories/new>.

Do not open a public issue for it. Include the TigerSetup version
(`tiger-setup --version`, or `tiger-setup inspect <Setup.exe>` for a built
installer), what you observed and how to reproduce it.

IT Tiger answers reports there, fixes confirmed issues in a new release, and
credits the reporter in the advisory unless asked not to.

## Supported versions

Security fixes are made in the current release only. A fix ships as a new
version; a published release is never rebuilt.
