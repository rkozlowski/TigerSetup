# ShareX 21.0.0 — benchmark functional contract

Derived from ShareX's real upstream Inno Setup script
(`ShareX.Setup/InnoSetup/ShareX-setup.iss`, read at the pinned tag). Only the
installer-level behavior below is in scope; ShareX itself creates its
`.sxcu`/`.sxie` file-type registrations and native-messaging-host entries at
first run, not at install time, so those are excluded (an installer contract,
not an application-runtime contract).

| Feature | Upstream default | Benchmark contract |
|---|---|---|
| Install scope | effectively either, via elevation (`PrivilegesRequired=none`, default dir `{commonpf}`) | both `user` and `machine` scopes, TigerSetup's native scope choice |
| Payload | full ShareX tree (~540 MB canonical) | identical canonical payload, all three technologies |
| Start Menu shortcut | always, app + uninstall | always, app shortcut (uninstall entry is each technology's own ARP/Start Menu convention) |
| Desktop shortcut | optional task, unchecked | optional, default **off** |
| Send To shortcut | optional task, unchecked | optional, default **off** |
| Explorer context menu ("Upload with ShareX") | optional task, unchecked | optional, default **off** |
| Run at Windows startup | optional task, unchecked | optional, default **off** |
| Uninstall registration (ARP) | Inno's own | each technology's native ARP registration |
| File associations / native messaging hosts | created by the app at first run | excluded (not installer behavior) |
| Browser extension support / disable PrintScreen | optional tasks in the real installer | excluded — narrow, ShareX-internal conveniences with no comparable typed primitive question across the three technologies; noted as a documented simplification, not scored |

Every option defaults to the same value as the real ShareX installer.
