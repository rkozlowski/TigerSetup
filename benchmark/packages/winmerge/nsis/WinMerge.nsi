; WinMerge 2.16.58(.2) benchmark package (NSIS 3.12). Functional contract:
; ../contract.md. Version uses the three-part "2.16.58" for identity parity.
;
; The Start Menu folder is fixed (${APP_NAME}), as it is in the Inno Setup
; package (DisableProgramGroupPage=yes) and the TigerSetup one: a chosen
; folder would have to be persisted for the uninstaller to find it again.

Unicode true

!define PAYLOAD_DIR "..\..\..\canonical\WinMerge\payload"
!define APP_NAME "WinMerge"
!define APP_VERSION "2.16.58"
!define APP_PUBLISHER "WinMerge Team (repackaged for the TigerSetup benchmark)"
!define APP_EXE "WinMergeU.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}"

!define MULTIUSER_EXECUTIONLEVEL Highest
!define MULTIUSER_MUI
!define MULTIUSER_INSTALLMODE_COMMANDLINE
!define MULTIUSER_INSTALLMODE_INSTDIR "${APP_NAME}"

!include "MUI2.nsh"
!include "MultiUser.nsh"
!include "FileFunc.nsh"
!include "WordFunc.nsh"
!include "WinMessages.nsh"
!include "LogicLib.nsh"

Name "${APP_NAME}"
OutFile "..\..\..\artifacts\winmerge\WinMerge-NSIS.exe"
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

Section "WinMerge" SecMain
  SetOutPath "$INSTDIR"
  File /r "${PAYLOAD_DIR}\*.*"

  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortCut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"

  WriteRegStr SHCTX "Software\Microsoft\Windows\CurrentVersion\App Paths\${APP_EXE}" "" "$INSTDIR\${APP_EXE}"
  WriteRegStr SHCTX "Software\Classes\.WinMerge" "" "WinMerge.Project.File"
  WriteRegStr SHCTX "Software\Classes\WinMerge.Project.File" "" "WinMerge Project File"
  WriteRegStr SHCTX "Software\Classes\WinMerge.Project.File\shell\open\command" "" '"$INSTDIR\${APP_EXE}" "%1"'

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

Section /o "Create a desktop icon" SecDesktop
  CreateShortCut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"
SectionEnd

Section "Add WinMerge to the Explorer context menu" SecContextMenu
  ; regsvr32 is resolved by CreateProcess's own search (System32), which is
  ; how every NSIS script that calls a Windows tool by name runs it.
  ExecWait 'regsvr32.exe /s "$INSTDIR\ShellExtensionX64.dll"'
SectionEnd

Section /o "Add WinMerge to PATH" SecPath
  ; NSIS has no built-in PATH primitive either (nor a bundled plugin for it
  ; in a stock install): same read-modify-write-and-broadcast this benchmark
  ; writes by hand for Inno Setup, ported to NSIS registry/system calls.
  Push $R0
  ${If} $MultiUser.InstallMode == "AllUsers"
    ReadRegStr $R0 HKLM "SYSTEM\CurrentControlSet\Control\Session Manager\Environment" "Path"
    WriteRegExpandStr HKLM "SYSTEM\CurrentControlSet\Control\Session Manager\Environment" "Path" "$R0;$INSTDIR"
  ${Else}
    ReadRegStr $R0 HKCU "Environment" "Path"
    WriteRegExpandStr HKCU "Environment" "Path" "$R0;$INSTDIR"
  ${EndIf}
  SendMessage ${HWND_BROADCAST} ${WM_WININICHANGE} 0 "STR:Environment" /TIMEOUT=5000
  Pop $R0
SectionEnd

Section "Uninstall"
  ExecWait 'regsvr32.exe /u /s "$INSTDIR\ShellExtensionX64.dll"'
  Push $R0
  ${If} $MultiUser.InstallMode == "AllUsers"
    ReadRegStr $R0 HKLM "SYSTEM\CurrentControlSet\Control\Session Manager\Environment" "Path"
  ${Else}
    ReadRegStr $R0 HKCU "Environment" "Path"
  ${EndIf}
  ${WordReplace} "$R0" ";$INSTDIR" "" "+" $R0
  ${WordReplace} "$R0" "$INSTDIR;" "" "+" $R0
  ${WordReplace} "$R0" "$INSTDIR" "" "+" $R0
  ${If} $MultiUser.InstallMode == "AllUsers"
    WriteRegExpandStr HKLM "SYSTEM\CurrentControlSet\Control\Session Manager\Environment" "Path" "$R0"
  ${Else}
    WriteRegExpandStr HKCU "Environment" "Path" "$R0"
  ${EndIf}
  SendMessage ${HWND_BROADCAST} ${WM_WININICHANGE} 0 "STR:Environment" /TIMEOUT=5000
  Pop $R0
  RMDir /r "$INSTDIR"
  RMDir /r "$SMPROGRAMS\${APP_NAME}"
  Delete "$DESKTOP\${APP_NAME}.lnk"
  DeleteRegKey SHCTX "Software\Microsoft\Windows\CurrentVersion\App Paths\${APP_EXE}"
  DeleteRegKey SHCTX "Software\Classes\.WinMerge"
  DeleteRegKey SHCTX "Software\Classes\WinMerge.Project.File"
  DeleteRegKey SHCTX "${UNINSTALL_KEY}"
SectionEnd
