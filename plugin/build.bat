@echo off
REM Build ts3plugin_tsn_win64.dll — run from a VS x64 developer prompt:
REM     cd plugin && build.bat
if not exist dist mkdir dist
cl /nologo /LD /O2 /MD /W3 /I include ts3plugin_tsn.c /Fe:dist\ts3plugin_tsn_win64.dll /link ws2_32.lib
if errorlevel 1 exit /b 1
echo Built plugin\dist\ts3plugin_tsn_win64.dll
