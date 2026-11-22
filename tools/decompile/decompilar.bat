@echo off
setlocal EnableDelayedExpansion
chcp 65001 >nul
title Decompilar mod.jar

rem ---------------------------------------------------------------------------
rem  Decompila todos os .jar da pasta deste .bat (ou o que voce arrastar nele).
rem
rem  Uso:
rem    1. Jogue este .bat na mesma pasta dos .jar e de um duplo clique.
rem    2. Ou arraste um .jar em cima dele.
rem
rem  Baixa o Vineflower automaticamente na primeira execucao (~1,6 MB) e
rem  reaproveita depois. O codigo sai em <pasta do bat>\decompilado\<nome do jar>\
rem
rem  Precisa do Java 17 ou mais novo (o Forge 1.18.2 ja exige isso).
rem ---------------------------------------------------------------------------

set "VINEFLOWER_URL=https://github.com/Vineflower/vineflower/releases/download/1.12.0/vineflower-1.12.0.jar"
set "VINEFLOWER=%TEMP%\vineflower-1.12.0.jar"
set "AQUI=%~dp0"
set "SAIDA=%AQUI%decompilado"

echo.
echo  ============================================================
echo   Decompilador de mods  -  Vineflower 1.12.0
echo  ============================================================
echo.

rem --- 1. Java instalado? ----------------------------------------------------
set "JAVA="
if defined JAVA_HOME if exist "%JAVA_HOME%\bin\java.exe" set "JAVA=%JAVA_HOME%\bin\java.exe"
if not defined JAVA (
    for /f "delims=" %%J in ('where java 2^>nul') do if not defined JAVA set "JAVA=%%J"
)
if not defined JAVA (
    echo  [ERRO] Java nao encontrado.
    echo.
    echo  Instale o Java 17 ou mais novo:
    echo    https://adoptium.net/
    echo.
    echo  Se voce roda o Forge 1.18.2, o Java 17 ja esta no computador:
    echo  procure por "java.exe" na pasta de instalacao do Minecraft e
    echo  acrescente a pasta "bin" dele ao PATH, ou defina JAVA_HOME.
    echo.
    goto :fim_erro
)
echo  Java: "%JAVA%"
for /f "tokens=3" %%V in ('"%JAVA%" -version 2^>^&1 ^| findstr /i "version"') do set "JAVAVER=%%~V"
echo  Versao: !JAVAVER!
echo.

rem --- 2. Vineflower: baixa se ainda nao existe -------------------------------
if exist "%VINEFLOWER%" goto :tem_vineflower

echo  Baixando o Vineflower (uma vez so)...
where curl >nul 2>&1
if errorlevel 1 goto :baixar_powershell
curl -L --fail --silent --show-error -o "%VINEFLOWER%" "%VINEFLOWER_URL%"
if errorlevel 1 goto :baixar_powershell
goto :tem_vineflower

:baixar_powershell
echo  curl indisponivel, tentando pelo PowerShell...
powershell -NoProfile -Command ^
  "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12;" ^
  "try { Invoke-WebRequest -Uri '%VINEFLOWER_URL%' -OutFile '%VINEFLOWER%' -UseBasicParsing } catch { exit 1 }"
if errorlevel 1 (
    echo.
    echo  [ERRO] Nao consegui baixar o Vineflower.
    echo  Baixe manualmente e salve como:
    echo    %VINEFLOWER%
    echo  Link: %VINEFLOWER_URL%
    echo.
    goto :fim_erro
)

:tem_vineflower
echo  Vineflower: %VINEFLOWER%
echo.

rem --- 3. Quais jars? ---------------------------------------------------------
set "ALVOS="
set "QTD=0"

if "%~1"=="" goto :procurar_pasta
rem Arrastado em cima do .bat: cada argumento e um caminho (pode ter espaco).
:arg_loop
if "%~1"=="" goto :tem_alvos
set "ALVOS=!ALVOS! "%~f1""
set /a QTD+=1
shift
goto :arg_loop

:procurar_pasta
for %%F in ("%AQUI%*.jar") do (
    set "ALVOS=!ALVOS! "%%~fF""
    set /a QTD+=1
)
if !QTD!==0 goto :sem_jars
goto :tem_alvos

:sem_jars
echo  [ERRO] Nenhum .jar encontrado em:
echo    %AQUI%
echo.
echo  Coloque este .bat na mesma pasta do .jar, ou arraste o .jar nele.
echo.
goto :fim_erro

:tem_alvos
echo  Jars encontrados: %QTD%
echo.

rem --- 4. Decompila cada um ---------------------------------------------------
set /a N=0
for %%F in (!ALVOS!) do (
    set /a N+=1
    set "NOME=%%~nF"
    set "DEST=%SAIDA%\!NOME!"
    echo  [!N%! de %QTD%] %%~nxF
    echo         -^> !DEST!

    if exist "!DEST!" rd /s /q "!DEST!"
    mkdir "!DEST!" 2>nul

    "%JAVA%" -Xmx2G -jar "%VINEFLOWER%" "%%~fF" "!DEST!"
    if errorlevel 1 echo         [FALHOU] veja a mensagem acima.
    if not errorlevel 1 call :contar "!DEST!"
    echo.
)

echo  ============================================================
echo   Pronto. Codigo em:
echo     %SAIDA%
echo  ============================================================
echo.
echo  Compacte aquela pasta em .zip e envie para o chat.
echo  Pode abrir em %SAIDA% agora? (S/N)
choice /c SN /n /m "> "
if errorlevel 2 goto :fim
start "" explorer "%SAIDA%"
goto :fim

rem --- Conta os .java gerados -------------------------------------------------
:contar
set "JQTD=0"
for /r "%~1" %%J in (*.java) do set /a JQTD+=1
echo         ok: !JQTD! arquivos .java
exit /b 0

:fim_erro
echo.
pause
exit /b 1

:fim
endlocal
