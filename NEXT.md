# Próxima atualização — estudar o TAC e aplicar

Documento de transferência para a próxima sessão. Leia isto primeiro; o resto do
repositório é a fonte de verdade sobre o código.

## Objetivo

Analisar o código-fonte decompilado do **Timeless and Classics (TAC)** — um mod
Forge 1.18.2 de armas, maduro e muito completo — e usar o que ele faz de bom como
referência de *design* para o Blackpowder Legacy.

Não é porte de código. Horizon tem API própria (`Hooks`, `Part`, `ClientHost`);
o TAC é Forge/NeoForge. O que se aproveita é **comportamento e números**, não
implementação. Ver "Licença" abaixo.

## Onde está o material

O usuário já rodou o decompilador. O resultado está no **Google Drive**, numa
pasta/zip chamada **"nova pasta"**.

O jar de origem é `tac-0.3.10.1-1.18.2.jar` (TAC 0.3.10.1 para Forge 1.18.2).

A conexão com o Google Drive está **ativada** para esta conversa. Antes de
qualquer coisa:

1. Liste o que existe no Drive (`list_connector_tools` com o slug `google_drive`,
   depois a ferramenta de listagem) e localize a "nova pasta".
2. Se for um zip, baixe e descompacte em algo como `/tmp/tac-src/` — **fora** do
   repositório, para não poluir o workspace nem o Git.
3. Se o download direto não rolar, peça ao usuário um link ou que anexe o zip.

## O que procurar no código do TAC

Em ordem de valor para este projeto:

1. **A mão em primeira pessoa.** O TAC desenha mãos animadas nos modelos das
   armas. Comparar com o que o `draw_forearm` faz hoje (desenha o braço vanilla
   do jogador). Vale ver: como ele resolve o pulso, se a mão é parte do modelo
   ou um modelo separado, e como sincroniza com o recuo.
2. **Curva de recuo.** Números reais: deslocamento, velocidade de retorno, se é
   acumulativo por tiro, se decai exponencial ou linearmente.
3. **Transição de ADS.** Quanto tempo leva, que easing usa, se mexe no FOV junto
   com a posição da arma, e como trata a mira (sight) alinhada com o eixo da
   câmera.
4. **Máquina de estados de recarga.** Estados, o que cancela o quê (trocar de
   item, atirar no meio), e como o servidor autoriza.
5. **Números de balanceamento.** Cadência, dano, alcance, capacidade, tempo de
   recarga.

Ao terminar, **escreva os achados neste arquivo**, numa seção "Achados", com
números e trechos curtos de referência. Não cole blocos grandes de código
decompilado no repositório.

## Estado atual do mod (v0.7.1)

Fonte de verdade: `src/lib.rs`. Resumo do que já existe:

| Área | Como está |
|---|---|
| ADS | `ADS_OFFSET = [-0.56, 0.38, 0.05]`; ease de 0.3/frame em `ADS_BITS`, compartilhado com FOV e vinheta. **Nunca foi calibrado visualmente.** |
| Vinheta | `GuiDraw::Blit` de `misc/vignette` com `Blend::Vignette`, brilho seguindo o ease do ADS |
| FOV | `1.0 - 0.4 * aim` em `compute_fov_modifier` |
| Recuo | `exp(-6p)` → z += 0.10, pitch += 7° |
| Lever | `LEVER_ANIM_TICKS = 8.0`; dip senoidal → roll += 10°, y −= 0.05 |
| Recarga | `RELOAD_TICKS = 6`; dip senoidal → y −= 0.07, x += 0.02, roll += 8° |
| Mão | Braço vanilla do jogador (`bake_layer` + `model_part`), pivô em `ARM_BACK`, punho em `GRIP_TEXELS` |
| Servidor | `LEVER_READY` (8 ticks entre tiros), `RELOAD_READY` (6 ticks entre cartuchos), autoridade total sobre o tubo |

### Pendências conhecidas

- **`ADS_OFFSET` nunca foi validado em jogo.** O usuário precisa mandar um
  screenshot mirando para calibrar os três eixos.
- **O braço em 1ª pessoa nunca foi visto em jogo.** Três coisas podem estar
  erradas: direção (`ARM_BACK`), rotação do punho (as três chamadas
  `rotate_about`) e posição do punho (`GRIP_TEXELS`). Um screenshot resolve.
- A textura do sprite é `minecraft:entity/player/wide/steve` — fixa. Se o
  jogador usar skin slim, a mão fica com a textura errada.

## Regras do projeto

- **Português**, técnico e objetivo, sem encher linguiça.
- **Validar pela CI, não no chute.** Não há toolchain Rust local no sandbox —
  `cargo` não existe. O único jeito de compilar é dar push e ver o Actions.
  Logs de job/run costumam falhar ao baixar; use as **annotations** do check run,
  que vêm por `api.github.com`:
  ```sh
  JOB=$(gh run view <run> --json jobs -q '.jobs[0].databaseId')
  CR=$(gh api "repos/celso-abreu/Blackpowder-Legacy/actions/jobs/$JOB" -q '.check_run_url' | sed 's|.*check-runs/||')
  gh api "repos/celso-abreu/Blackpowder-Legacy/check-runs/$CR/annotations" -q '.[].message'
  ```
- **Download de logs não funciona** no sandbox (EOF do blob storage). Não perca
  tempo tentando.
- **`gh run rerun` está quebrado** neste repo ("workflow file may be broken").
  Para rodar de novo, faça um commit vazio e dê push.
- **Não edite o `[wasm]` do `horizon-mod.toml` à mão.** O
  `cargo horizon-mod build --wasm` escreve.
- O SDK é pinado (`0a875da91dbb0fdcb7ba8c42c11341aa229af51a`). A API muda entre
  commits: confira assinaturas no SDK antes de inventar nomes.

## Licença — leia antes de copiar

O **TAC é All Rights Reserved**. Estudar para entender comportamento é legítimo;
copiar código não é. Regras práticas:

- Números, curvas e ideias de design: pode usar.
- Código Java do TAC: **não** entra neste repositório, nem traduzido linha a
  linha.
- Reimplemente contra a API do Horizon, do zero.
- Se alguma coisa ficar parecida demais com o original, avise o usuário.

Também vale para o usuário: o código decompilado não deve ir para o repositório
público. O `.gitignore` já cobre `decompilado/` e `decompilado-cfr/`.

## Fluxo de trabalho

1. Ler este documento e o `README.md`.
2. Verificar em que branch está (`git log --oneline -3`, `git status`).
3. Pegar o material do Drive.
4. Analisar e escrever os achados neste arquivo.
5. Propor as mudanças ao usuário **antes** de implementar — ele prefere decidir
   o escopo.
6. Implementar, subir, esperar a CI, abrir PR.
7. Depois do merge: tag `v0.8.0` → a release é publicada sozinha.

## Verificação rápida no início da sessão

```sh
cd /home/user/Blackpowder-Legacy
git log --oneline -3          # espera-se algo a partir de v0.7.1
grep '^version' Cargo.toml horizon-mod.toml   # devem bater entre si
ls tools/decompile/           # scripts de decompilação, se precisar de novo
```
