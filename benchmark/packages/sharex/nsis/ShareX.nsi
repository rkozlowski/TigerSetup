; ShareX 21.0.0 benchmark package (NSIS 3.12). Functional contract:
; ../contract.md. Payload is the canonical ShareX payload, referenced
; relative to this script.
;
; Compression: LZMA, solid -- the benchmark's production-quality setting for
; all three technologies (see benchmark/README.md).
;
; Per-user/per-machine choice: stock Inno Setup has this as one config line
; (PrivilegesRequiredOverridesAllowed=dialog); NSIS has no built-in
; equivalent and needs the standard MultiUser.nsh pattern (its own wizard
; page, an .onInit re-exec-elevated dance, and per-mode path/shortcut
; plumbing) to reach the same user-visible behavior. That extra scripting
; is itself a benchmark data point, reported in report.md.
;
; The Start Menu folder is fixed (${APP_NAME}), as it is in the Inno Setup
; package (DisableProgramGroupPage=yes) and the TigerSetup one: a chosen
; folder would have to be persisted for the uninstaller to find it again.

Unicode true

!define PAYLOAD_DIR "..\..\..\canonical\ShareX\payload"
!define APP_NAME "ShareX"
!define APP_VERSION "21.0.0"
!define APP_PUBLISHER "ShareX Team (repackaged for the TigerSetup benchmark)"
!define APP_EXE "ShareX.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}"

!define MULTIUSER_EXECUTIONLEVEL Highest
!define MULTIUSER_MUI
!define MULTIUSER_INSTALLMODE_COMMANDLINE
!define MULTIUSER_INSTALLMODE_INSTDIR "${APP_NAME}"

!include "MUI2.nsh"
!include "MultiUser.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"

Name "${APP_NAME}"
OutFile "..\..\..\artifacts\sharex\ShareX-NSIS.exe"
RequestExecutionLevel highest
SetCompressor /SOLID lzma

VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey "ProductName" "${APP_NAME}"
VIAddVersionKey "ProductVersion" "${APP_VERSION}"
VIAddVersionKey "CompanyName" "${APP_PUBLISHER}"
VIAddVersionKey "FileVersion" "${APP_VERSION}.0"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MULTIUSER_PAGE_INSTALLMODE
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ; MULTIUSER_INIT sets $INSTDIR to the mode's default and would discard a
  ; /D= given on the command line, which NSIS applies before .onInit; the
  ; standard idiom keeps what /D= chose.
  StrCpy $R9 $INSTDIR
  !insertmacro MULTIUSER_INIT
  ${If} $R9 != ""
    StrCpy $INSTDIR $R9
  ${EndIf}
FunctionEnd

Function un.onInit
  !insertmacro MULTIUSER_UNINIT
FunctionEnd

Section "ShareX" SecMain
  SetOutPath "$INSTDIR"
  File /r "${PAYLOAD_DIR}\*.*"

  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortCut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"

  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ; The uninstall string carries the install mode, so the uninstaller
  ; Add/Remove Programs starts works in the hive and folders this install
  ; used rather than MultiUser's default.
  WriteRegStr SHCTX "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr SHCTX "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe" /$MultiUser.InstallMode'
  WriteRegStr SHCTX "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /$MultiUser.InstallMode /S'
  WriteRegStr SHCTX "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE}"
  WriteRegStr SHCTX "${UNINSTALL_KEY}" "Publisher" "${APP_PUBLISHER}"
  WriteRegStr SHCTX "${UNINSTALL_KEY}" "DisplayVersion" "${APP_VERSION}"
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD SHCTX "${UNINSTALL_KEY}" "EstimatedSize" "$0"
  WriteRegDWORD SHCTX "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD SHCTX "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section /o "Create a desktop shortcut" SecDesktop
  CreateShortCut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"
SectionEnd

Section /o "Add a Send To entry" SecSendTo
  CreateShortCut "$SENDTO\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"
SectionEnd

Section /o 'Add "Upload with ShareX" to the Explorer context menu' SecContextMenu
  WriteRegStr SHCTX "Software\Classes\*\shell\ShareX" "" "Upload with ShareX"
  WriteRegStr SHCTX "Software\Classes\*\shell\ShareX" "Icon" "$INSTDIR\${APP_EXE}"
  WriteRegStr SHCTX "Software\Classes\*\shell\ShareX\command" "" '"$INSTDIR\${APP_EXE}" -workflow "%1"'
SectionEnd

Section /o "Start ShareX at sign-in" SecStartup
  CreateShortCut "$SMSTARTUP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "-silent"
SectionEnd

Section "Uninstall"
  RMDir /r "$INSTDIR"
  RMDir /r "$SMPROGRAMS\${APP_NAME}"
  Delete "$DESKTOP\${APP_NAME}.lnk"
  Delete "$SENDTO\${APP_NAME}.lnk"
  Delete "$SMSTARTUP\${APP_NAME}.lnk"
  DeleteRegKey SHCTX "Software\Classes\*\shell\ShareX"
  DeleteRegKey SHCTX "${UNINSTALL_KEY}"
SectionEnd
