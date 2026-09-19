; qBittorrent 5.2.3 benchmark package (NSIS 3.12). Functional contract:
; ../contract.md. Machine-scope only, matching the real installer's actual
; (HKLM-only) behavior; SetShellVarContext all puts the shortcuts in the
; shared folders, as the real installer does.

Unicode true
!include "MUI2.nsh"
!include "FileFunc.nsh"

!define PAYLOAD_DIR "..\..\..\canonical\qBittorrent\payload"
!define APP_NAME "qBittorrent"
!define APP_VERSION "5.2.3"
!define APP_PUBLISHER "qBittorrent Team (repackaged for the TigerSetup benchmark)"
!define APP_EXE "qbittorrent.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}"

Name "${APP_NAME}"
OutFile "..\..\..\artifacts\qbittorrent\qBittorrent-NSIS.exe"
InstallDir "$PROGRAMFILES64\${APP_NAME}"
RequestExecutionLevel admin
SetCompressor /SOLID lzma

VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey "ProductName" "${APP_NAME}"
VIAddVersionKey "ProductVersion" "${APP_VERSION}"
VIAddVersionKey "CompanyName" "${APP_PUBLISHER}"
VIAddVersionKey "FileVersion" "${APP_VERSION}.0"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

; A stock NSIS installer is a 32-bit process; without SetRegView 64 its
; HKLM\Software writes land in WOW6432Node, where a 64-bit application's
; Add/Remove Programs entry and App Paths do not belong. The real qBittorrent
; and VLC x64 installers select the 64-bit view the same way.
Function .onInit
  SetRegView 64
  SetShellVarContext all
FunctionEnd

Function un.onInit
  SetRegView 64
  SetShellVarContext all
FunctionEnd

Section "qBittorrent" SecMain
  SetOutPath "$INSTDIR"
  File /r "${PAYLOAD_DIR}\*.*"

  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortCut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"

  WriteRegStr HKLM "Software\Classes\.torrent" "" "qBittorrent.File.Torrent"
  WriteRegStr HKLM "Software\Classes\qBittorrent.File.Torrent" "" "BitTorrent file"
  WriteRegStr HKLM "Software\Classes\qBittorrent.File.Torrent\shell\open\command" "" '"$INSTDIR\${APP_EXE}" "%1"'
  WriteRegStr HKLM "Software\Classes\magnet" "" "URL:Magnet URI"
  WriteRegStr HKLM "Software\Classes\magnet" "URL Protocol" ""
  WriteRegStr HKLM "Software\Classes\magnet\shell\open\command" "" '"$INSTDIR\${APP_EXE}" "%1"'

  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKLM "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "Publisher" "${APP_PUBLISHER}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayVersion" "${APP_VERSION}"
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "EstimatedSize" "$0"
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section /o "Create a desktop shortcut" SecDesktop
  CreateShortCut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"
SectionEnd

Section /o "Start qBittorrent on Windows startup" SecStartup
  ; A link in the shared Startup folder, the mechanism all three benchmark
  ; packages use (contract.md).
  CreateShortCut "$SMSTARTUP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"
SectionEnd

Section "Add a firewall exception for qBittorrent" SecFirewall
  ExecWait 'netsh.exe advfirewall firewall add rule name="qBittorrent" dir=in action=allow program="$INSTDIR\${APP_EXE}" enable=yes'
SectionEnd

Section "Disable the Windows path length limit" SecLongPaths
  WriteRegDWORD HKLM "SYSTEM\CurrentControlSet\Control\FileSystem" "LongPathsEnabled" 1
SectionEnd

Section "Uninstall"
  ExecWait 'netsh.exe advfirewall firewall delete rule name="qBittorrent"'
  Delete "$SMSTARTUP\${APP_NAME}.lnk"
  RMDir /r "$INSTDIR"
  RMDir /r "$SMPROGRAMS\${APP_NAME}"
  Delete "$DESKTOP\${APP_NAME}.lnk"
  DeleteRegKey HKLM "Software\Classes\.torrent"
  DeleteRegKey HKLM "Software\Classes\qBittorrent.File.Torrent"
  DeleteRegKey HKLM "Software\Classes\magnet"
  DeleteRegKey HKLM "${UNINSTALL_KEY}"
SectionEnd
