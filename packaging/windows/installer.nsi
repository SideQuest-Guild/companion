; SideQuest Companion setup for Windows. Built by the release workflow with:
;   makensis -DVERSION=X.Y.Z -DARCH=x86_64 -DEXE=<path to sidequest-companion.exe> -DICON=<path to icon.ico> \
;            -DOUTFILE=<setup.exe path> packaging/windows/installer.nsi
;
; Installs for the current user only (no admin prompt) into %LOCALAPPDATA%\Programs\SideQuest Companion, where the
; app's own self-updater can replace the exe later. Closes a running copy first, wherever it was started from, so a
; newer setup always replaces an older app. Settings, sign-in and logs live in %APPDATA% and are kept on uninstall.

Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user
ManifestDPIAware true

!define APP_NAME "SideQuest Companion"
!define APP_EXE "sidequest-companion.exe"
!define PUBLISHER "SideQuest Guild"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\SideQuestCompanion"
; Written by the app's "Launch at login" setting (the auto-launch crate uses the app name as the value name).
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define STARTUP_APPROVED_KEY "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"

Name "${APP_NAME}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APP_NAME}"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
BrandingText "${APP_NAME} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP_NAME}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${APP_NAME} setup"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "LegalCopyright" "Copyright (c) 2026 ${PUBLISHER}"

!include "MUI2.nsh"
!include "LogicLib.nsh"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TEXT "This installs ${APP_NAME} ${VERSION}.$\r$\n$\r$\nIf the app is running, setup closes it first and replaces it with this version. Your sign-in and settings are kept."
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Open ${APP_NAME}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; Ask a running app to quit through its own single-instance port (`--quit`, from 0.3.4 on), then end any copy that
; is still running: older versions ignore the request, and a copy unzipped elsewhere shares the same exe name.
!macro CLOSE_RUNNING_APP QUIT_EXE
  ; ExecWait leaves $0 alone when it can't start the exe, so start from "still running".
  StrCpy $0 1
  ExecWait '"${QUIT_EXE}" --quit' $0
  ${If} $0 != 0
    nsExec::Exec 'taskkill /F /T /IM "${APP_EXE}"'
    Pop $0
  ${EndIf}
  ; Windows can hold the exe for a moment after its process exits.
  Sleep 1000
!macroend

Section "Install"
  DetailPrint "Closing ${APP_NAME} if it is running..."
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File "/oname=${APP_EXE}" "${EXE}"
  !insertmacro CLOSE_RUNNING_APP "$PLUGINSDIR\${APP_EXE}"

  SetOutPath "$INSTDIR"
  File "/oname=${APP_EXE}" "${EXE}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"

  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "https://sidequest-guild.com"
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1

  ; If "Launch at login" pointed at a copy unzipped somewhere else, point it here instead.
  ReadRegStr $0 HKCU "${RUN_KEY}" "${APP_NAME}"
  ${If} $0 != ""
    WriteRegStr HKCU "${RUN_KEY}" "${APP_NAME}" '"$INSTDIR\${APP_EXE}" --hidden'
  ${EndIf}
SectionEnd

Section "Uninstall"
  DetailPrint "Closing ${APP_NAME} if it is running..."
  !insertmacro CLOSE_RUNNING_APP "$INSTDIR\${APP_EXE}"

  Delete "$INSTDIR\${APP_EXE}"
  ; The self-updater (self-replace) can leave `.sidequest-companion.<random>.__relocated__.exe` behind.
  Delete "$INSTDIR\.sidequest-companion.*.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP_NAME}.lnk"

  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegValue HKCU "${RUN_KEY}" "${APP_NAME}"
  DeleteRegValue HKCU "${STARTUP_APPROVED_KEY}" "${APP_NAME}"
SectionEnd
