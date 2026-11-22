@echo off
chcp 65001 >nul
title Decompilar mod.jar (CFR)

rem ---------------------------------------------------------------------------
rem  Alternativa ao decompilar.bat, usando o CFR.
rem
rem  Use este se o Vineflower travar, der erro, ou gerar Java incompreensivel
rem  em algum arquivo. O CFR costuma acertar coisas que o outro erra, e vice-versa.
rem
rem  Mesmo uso: jogue o .bat na pasta dos .jar e de duplo clique, ou arraste
rem  um .jar em cima dele. Precisa do Java 17+.
rem ---------------------------------------------------------------------------

set "CFR_URL=https://github.com/leibnitz27/cfr/releases/download/0.152/cfr-0.152.jar"
set "CFR=%TEMP%\cfr-0.152.jar"
set "AQUI=%~dp0"
set "SAIDA=%AQUI%decompilado-cfr"

echo.
echo  ============================================================
echo   Decompilador de mods  -  CFR 0.152
echo  ============================================================
echo.

set "JAVA="
if defined JAVA_HOME if exist "%JAVA_HOME%\bin\java.exe" set "JAVA=%JAVA_HOME%\bin\java.exe"
if not defined JAVA (
    for /f "delims=" %%J in ('where java 2^>nul') do if not defined JAVA set "JAVA=%%J"
)
if not defined JAVA (
    echo  [ERRO] Java nao encontrado. Instale o Java 17+:
    echo    https://adoptium.net/
    echo.
    goto :fim_erro
)
echo  Java: "%JAVA%"
echo.

if exist "%CFR%" goto :tem_cfr
echo  Baixando o CFR (uma vez so)...
where curl >nul 2>&1
if errorlevel 1 goto :baixar_powershell
curl -L --fail --silent --show-error -o "%CFR%" "%CFR_URL%"
if errorlevel 1 goto :baixar_powershell
goto :tem_cfr

:baixar_powershell
powershell -NoProfile -Command ^
  "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12;" ^
  "try { Invoke-WebRequest -Uri '%CFR_URL%' -OutFile '%CFR%' -UseBasicParsing } catch { exit 1 }"
if errorlevel 1 (
    echo.
    echo  [ERRO] Nao consegui baixar o CFR.
    echo  Baixe manualmente e salve como: %CFR%
    echo  Link: %CFR_URL%
    echo.
    goto :fim_erro
)

:tem_cfr
echo  CFR: %CFR%
echo.

rem --- alvos ----------------------------------------------------------------
set "TEM_ARG=0"
if not "%~1"=="" set "TEM_ARG=1"

if exist "%SAIDA%" rd /s /q "%SAIDA%"
mkdir "%SAIDA%" 2>nul

if "%TEM_ARG%"=="1" (
    "%JAVA%" -Xmx2G -jar "%CFR%" "%~f1" --outputdir "%SAIDA%" --comments false
    if errorlevel 1 (
        echo.
        echo  [FALHOU] veja a mensagem acima.
        goto :fim_erro
    )
) else (
    set "QTD=0"
    for %%F in ("%AQUI%*.jar") do set /a QTD+=1
    if "!QTD!"=="0" (
        echo  [ERRO] Nenhum .jar encontrado em:
        echo    %AQUI%
        echo  Coloque este .bat na pasta do .jar, ou arraste o .jar nele.
        echo.
        goto :fim_erro
    )
    echo  Jars encontrados: !QTD!
    echo.
    for %%F in ("%AQUI%*.jar") do (
        echo  -^> %%~nxF
        "%JAVA%" -Xmx2G -jar "%CFR%" "%%~fF" --outputdir "%SAIDA%\%%~nF" --comments false
        if errorlevel 1 echo     [FALHOU] veja a mensagem acima.
        echo.
    )
)

set "JQTD=0"
for /r "%SAIDA%" %%J in (*.java) do set /a JQTD+=1
echo  ============================================================
echo   Pronto: !JQTD! arquivos .java em
echo     %SAIDA%
echo  ============================================================
echo.
echo  Compacte aquela pasta em .zip e envie para o chat.
echo.
goto :fim

:fim_erro
echo.
pause
exit /b 1

:fim
pause
