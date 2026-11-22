# Decompilar um mod .jar

Ferramentas de **estudo**, não fazem parte do mod e não entram no `.hmod`.

Dois scripts para Windows que baixam o decompilador e transformam `.jar` em
código Java legível. Não precisa instalar nada além do Java.

## Qual usar

| Script | Decompilador | Quando |
|---|---|---|
| `decompilar.bat` | Vineflower 1.12.0 | **Padrão.** Melhor resultado em código moderno. |
| `decompilar-cfr.bat` | CFR 0.152 | Quando o Vineflower falha, ou gera Java estranho num arquivo. |

Os dois podem ser usados no mesmo jar: um acerta o que o outro erra.

## Uso

1. Coloque o `.bat` na **mesma pasta** do `.jar`.
2. Dê **duplo clique** no `.bat`.
3. Espere. Na primeira vez ele baixa o decompilador (~1,6 MB) e guarda em
   `%TEMP%`, então não baixa de novo.

O código sai em `decompilado/<nome-do-jar>/`, ao lado do `.bat`, e o script
pergunta se você quer abrir a pasta. Essa saída já está no `.gitignore` da raiz
(`decompilado/`, `decompilado-cfr/`), então não corre risco de ir para o Git.

Também funciona **arrastando** um ou mais `.jar` em cima do `.bat` — aí ele
decompila só aqueles, mesmo que estejam em outra pasta.

## Se der erro de Java

O Forge 1.18.2 já exige Java 17, então ele está no computador — o script só
pode não estar achando. Duas saídas:

- Instale o Java 17+ de https://adoptium.net/ (instala no PATH sozinho)
- Ou defina a variável `JAVA_HOME` apontando para a pasta onde o `java.exe` está

## Se a decompilação falhar

O script mostra a mensagem do Java. As causas comuns:

- **Java antigo** — o mod é Java 17, precisa de Java 17 ou mais novo
- **Pouca memória** — mods grandes pedem mais que 2 GB; edite `-Xmx2G` para `-Xmx4G`
- **Jar corrompido ou muito grande** — tente o outro script

## Depois

Compacte a pasta `decompilado/` em `.zip` e envie no chat.

Se o zip ficar grande demais, envie só o necessário:

- a classe base da arma (`*GunItem*.java`, `*Gun*.java`)
- o handler de tiro, recuo e projétil
- o renderer e as classes de animação
- a classe de config
- o `mods.toml` do jar original

## Aviso

Não guarde o código decompilado num repositório público. Ele pertence ao autor
do mod — serve para estudo, não para redistribuição.
