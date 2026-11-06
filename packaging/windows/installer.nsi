Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"

!ifndef APP_VERSION
  !error "APP_VERSION is required. Use packaging/windows/build.ps1."
!endif
!ifndef APP_VERSION_NUMERIC
  !error "APP_VERSION_NUMERIC is required."
!endif
!ifndef APP_EXE_PATH
  !error "APP_EXE_PATH is required."
!endif
!ifndef ICON_PATH
  !error "ICON_PATH is required."
!endif
!ifndef LICENSE_PATH
  !error "LICENSE_PATH is required."
!endif
!ifndef OUTPUT_FILE
  !error "OUTPUT_FILE is required."
!endif

!define APP_NAME "MangaMeeya Rust"
!define APP_EXE "mangameeya-rust.exe"
!define APP_KEY "Software\io.github.mangameeya.rust"
!define PROG_ID "MangaMeeyaRust.Comic"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\io.github.mangameeya.rust"
!define APP_PATH_KEY "Software\Microsoft\Windows\CurrentVersion\App Paths\${APP_EXE}"

Name "${APP_NAME}"
OutFile "${OUTPUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\${APP_NAME}"
InstallDirRegKey HKCU "${APP_KEY}" "InstallDir"
RequestExecutionLevel user
ManifestDPIAware true
SetCompressor /SOLID lzma
VIProductVersion "${APP_VERSION_NUMERIC}"
VIAddVersionKey /LANG=1033 "ProductName" "${APP_NAME}"
VIAddVersionKey /LANG=1033 "FileDescription" "${APP_NAME} installer"
VIAddVersionKey /LANG=1033 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "Copyright 2026 MangaMeeya Rust contributors"

!define MUI_ICON "${ICON_PATH}"
!define MUI_UNICON "${ICON_PATH}"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_LICENSE "${LICENSE_PATH}"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!macro RegisterExtension EXT
  ; Advertise Open With support. Do not write the extension's default value or UserChoice.
  WriteRegStr HKCU "Software\Classes\.${EXT}\OpenWithProgids" "${PROG_ID}" ""
  WriteRegStr HKCU "Software\Classes\Applications\${APP_EXE}\SupportedTypes" ".${EXT}" ""
  WriteRegStr HKCU "${APP_KEY}\Capabilities\FileAssociations" ".${EXT}" "${PROG_ID}"
!macroend

!macro UnregisterExtension EXT
  DeleteRegValue HKCU "Software\Classes\.${EXT}\OpenWithProgids" "${PROG_ID}"
  DeleteRegKey /ifempty HKCU "Software\Classes\.${EXT}\OpenWithProgids"
  DeleteRegKey /ifempty HKCU "Software\Classes\.${EXT}"
!macroend

Function .onInit
  SetShellVarContext current
FunctionEnd

Function un.onInit
  SetShellVarContext current
FunctionEnd

Section "Install"
  SetOutPath "$INSTDIR"
  File /oname=mangameeya-rust.exe "${APP_EXE_PATH}"
  File /oname=LICENSE.txt "${LICENSE_PATH}"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortCut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"

  WriteRegStr HKCU "${APP_KEY}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${APP_PATH_KEY}" "" "$INSTDIR\${APP_EXE}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "MangaMeeya Rust contributors"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE},0"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1

  WriteRegStr HKCU "Software\Classes\${PROG_ID}" "" "Comic book or image"
  WriteRegStr HKCU "Software\Classes\${PROG_ID}\DefaultIcon" "" '$\"$INSTDIR\${APP_EXE}$\",0'
  WriteRegStr HKCU "Software\Classes\${PROG_ID}\shell\open\command" "" '$\"$INSTDIR\${APP_EXE}$\" $\"%1$\"'
  WriteRegStr HKCU "Software\Classes\Applications\${APP_EXE}" "FriendlyAppName" "${APP_NAME}"
  WriteRegStr HKCU "Software\Classes\Applications\${APP_EXE}\shell\open\command" "" '$\"$INSTDIR\${APP_EXE}$\" $\"%1$\"'
  WriteRegStr HKCU "${APP_KEY}\Capabilities" "ApplicationName" "${APP_NAME}"
  WriteRegStr HKCU "${APP_KEY}\Capabilities" "ApplicationDescription" "Read image folders and comic archives"
  WriteRegStr HKCU "Software\RegisteredApplications" "${APP_NAME}" "${APP_KEY}\Capabilities"
  !insertmacro RegisterExtension "jpg"
  !insertmacro RegisterExtension "jpeg"
  !insertmacro RegisterExtension "jpe"
  !insertmacro RegisterExtension "png"
  !insertmacro RegisterExtension "gif"
  !insertmacro RegisterExtension "bmp"
  !insertmacro RegisterExtension "webp"
  !insertmacro RegisterExtension "zip"
  !insertmacro RegisterExtension "cbz"
  !insertmacro RegisterExtension "rar"
  !insertmacro RegisterExtension "cbr"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

Section "Uninstall"
  ClearErrors
  Delete "$INSTDIR\${APP_EXE}"
  ${If} ${Errors}
    MessageBox MB_OK|MB_ICONSTOP "Close ${APP_NAME}, then run the uninstaller again." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\Uninstall.exe"
  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk"
  RMDir "$SMPROGRAMS\${APP_NAME}"
  RMDir "$INSTDIR"

  !insertmacro UnregisterExtension "jpg"
  !insertmacro UnregisterExtension "jpeg"
  !insertmacro UnregisterExtension "jpe"
  !insertmacro UnregisterExtension "png"
  !insertmacro UnregisterExtension "gif"
  !insertmacro UnregisterExtension "bmp"
  !insertmacro UnregisterExtension "webp"
  !insertmacro UnregisterExtension "zip"
  !insertmacro UnregisterExtension "cbz"
  !insertmacro UnregisterExtension "rar"
  !insertmacro UnregisterExtension "cbr"
  DeleteRegKey HKCU "Software\Classes\${PROG_ID}"
  DeleteRegKey HKCU "Software\Classes\Applications\${APP_EXE}"
  DeleteRegValue HKCU "Software\RegisteredApplications" "${APP_NAME}"
  DeleteRegKey HKCU "${APP_PATH_KEY}"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "${APP_KEY}"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  ; Keep the user's reading history and preferences. Never remove shared extension keys.
SectionEnd
