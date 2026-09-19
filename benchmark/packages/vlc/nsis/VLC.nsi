; VLC 3.0.23 benchmark package (NSIS 3.12). Functional contract:
; ../contract.md. Machine-scope only, matching the real installer, whose
; SetShellVarContext all puts the shortcuts in the shared folders.

Unicode true
!include "MUI2.nsh"
!include "FileFunc.nsh"

!define PAYLOAD_DIR "..\..\..\canonical\VLC\payload"
!define APP_NAME "VLC media player"
!define APP_VERSION "3.0.23"
!define APP_PUBLISHER "VideoLAN (repackaged for the TigerSetup benchmark)"
!define APP_EXE "vlc.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}"

Name "${APP_NAME}"
OutFile "..\..\..\artifacts\vlc\VLC-NSIS.exe"
InstallDir "$PROGRAMFILES64\VideoLAN\VLC"
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

; The representative file-association subset (contract.md): always on.
!macro Associate ext desc
  WriteRegStr HKLM "Software\Classes\.${ext}" "" "VLC.${ext}"
  WriteRegStr HKLM "Software\Classes\VLC.${ext}" "" "${desc}"
  WriteRegStr HKLM "Software\Classes\VLC.${ext}\shell\open\command" "" '"$INSTDIR\${APP_EXE}" "%1"'
!macroend

; "Play with VLC" on the associated types: the optional context menu.
!macro PlayWithVlc ext
  WriteRegStr HKLM "Software\Classes\VLC.${ext}\shell\PlayWithVLC" "" "Play with VLC"
  WriteRegStr HKLM "Software\Classes\VLC.${ext}\shell\PlayWithVLC\command" "" '"$INSTDIR\${APP_EXE}" "%1"'
!macroend

!macro Unassociate ext
  DeleteRegKey HKLM "Software\Classes\.${ext}"
  DeleteRegKey HKLM "Software\Classes\VLC.${ext}"
!macroend

Section "VLC media player" SecMain
  SetOutPath "$INSTDIR"
  File /r "${PAYLOAD_DIR}\*.*"

  CreateDirectory "$SMPROGRAMS\VideoLAN"
  CreateShortCut "$SMPROGRAMS\VideoLAN\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"

  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\App Paths\${APP_EXE}" "" "$INSTDIR\${APP_EXE}"

  !insertmacro Associate "mp3" "MP3 audio file"
  !insertmacro Associate "flac" "FLAC audio file"
  !insertmacro Associate "wav" "WAV audio file"
  !insertmacro Associate "mp4" "MP4 video file"
  !insertmacro Associate "mkv" "Matroska video file"
  !insertmacro Associate "avi" "AVI video file"

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

Section 'Add "Play with VLC" to the Explorer context menu' SecContextMenu
  !insertmacro PlayWithVlc "mp3"
  !insertmacro PlayWithVlc "flac"
  !insertmacro PlayWithVlc "wav"
  !insertmacro PlayWithVlc "mp4"
  !insertmacro PlayWithVlc "mkv"
  !insertmacro PlayWithVlc "avi"
  WriteRegStr HKLM "Software\Classes\Directory\Background\shell\PlayWithVLC" "" "Play with VLC"
  WriteRegStr HKLM "Software\Classes\Directory\Background\shell\PlayWithVLC\command" "" '"$INSTDIR\${APP_EXE}" "%V"'
SectionEnd

Section "Uninstall"
  !insertmacro Unassociate "mp3"
  !insertmacro Unassociate "flac"
  !insertmacro Unassociate "wav"
  !insertmacro Unassociate "mp4"
  !insertmacro Unassociate "mkv"
  !insertmacro Unassociate "avi"
  DeleteRegKey HKLM "Software\Classes\Directory\Background\shell\PlayWithVLC"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\App Paths\${APP_EXE}"
  RMDir /r "$INSTDIR"
  RMDir /r "$SMPROGRAMS\VideoLAN"
  Delete "$DESKTOP\${APP_NAME}.lnk"
  DeleteRegKey HKLM "${UNINSTALL_KEY}"
SectionEnd
