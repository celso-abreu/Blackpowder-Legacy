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

## Achados

Material analisado em `/tmp/tac-src/` (`Nova pasta.zip` do Drive →
`tac-0.3.10.1-1.18.2.jar` + árvore CFR com 1.304 classes) e cruzado com o SDK
pinado do Horizon (`0a875da91dbb0fdcb7ba8c42c11341aa229af51a`).

### 1. Mão em primeira pessoa (TAC + 3 bugs reais achados no `draw_forearm` atual)

- **Como o TAC resolve (`RenderUtil`, `PlayerHandAnimation`, `TwoHandedPoseHighRes_m1894`):**
  - Os braços **não** fazem parte do JSON da arma; o mod desenha o `ModelPart`
    do jogador (`rightArm` + `rightSleeve` e `leftArm` + `leftSleeve`) no
    espaço local da arma, herdando a matriz já transformada por ADS, recuo e
    animação.
  - Pose procedural de carabina lever-action (`TwoHandedPoseHighRes_m1894`,
    após `translate(0, 0, -1)` e `rotY(180°)`):
    - **Mão esquerda (guarda-mão):** `(7 * side * 0.0625, -0.995, -0.2)`,
      rotações `X=80°`, `Y=-15°*side`, `Z=-15°*side`, `X=-35°`, escala `1.0`.
      Durante a recarga (`r`), desliza `(+1.5r, -1.0r, -1.5r) * 0.0625`.
    - **Mão direita (punho):** `(-2.5 * 0.0465, -0.4, -0.975)`, rotação
      `X=80°`, escala `0.5`.
- **Diagnóstico da v0.7.1 contra o código do Horizon (`horizon-client` / `horizon-mod-vocab`):**
  1. **Árvore inteira em vez do braço:** `client_setup` guarda o retorno bruto
     de `host.bake_layer("minecraft:player#main")` (raiz com cabeça, tronco,
     pernas e braços) e `draw_forearm` desenha essa raiz sem chamar
     `.get_child("right_arm")` / `.get_child("right_sleeve")` nem zerar o
     `initial_pose` do ombro (`x = -5, y = 2, z = 0` texels).
  2. **`event.draw` não herda `event.render`:** em
     `crates/horizon-client/src/game/interface/frame_geometry.rs` e
     `hooks/render_hand.rs`, `render_hand` roda **antes** de
     `hand_pose_matrix(render)` e **antes** de `item_pose(0.56, -0.52 - equip*0.6, -0.72)`.
     O comentário em `src/lib.rs` assumia que `draw` seguia `render` de graça,
     mas `draw` tem pose própria: na v0.7.1 o braço fica parado no quadril
     enquanto a arma mira, recua ou cicla. É preciso replicar a pose da arma
     em `event.draw` antes de posicionar punho e guarda-mão.
  3. **Sistema de eixos do `ModelPart`:** `host/draw.rs` documenta que
     `ModelPart` usa `+Y` para baixo e frente em `-Z`, exigindo
     `draw.scale([1.0, -1.0, -1.0])` antes de `model_part`; sem isso o braço
     fica invertido verticalmente.
  - **Skin do jogador no WASM:** `hooks/render_hand.rs` confirma que o estado
    de render do jogador não é exposto aos mods WASM no commit pinado; o sprite
    continua fixo em `"minecraft:entity/player/wide/steve"`, mas já podemos
    extrair `right_arm` + `right_sleeve` e `left_arm` + `left_sleeve`.

### 2. Curva de recuo (`RecoilHandler`, `GunRenderingHandler`, `SecondOrderDynamics`)

- O TAC separa o recuo em **câmera** e **modelo**, nunca usando apenas decaimento
  exponencial simples:
  - **Recuo da câmera (`RecoilHandler`):**
    - Magnitude por tiro: `angle = recoilAngle * (1 - adsReduction * ads) * rand(0.75..1.22)`.
    - **3 fases:** subida rápida de pitch nos primeiros ~`100 ms`; platô curto
      de `100 ms`; retorno parcial a `1 / 1.75` (~`57%`) da velocidade de
      subida, deixando ~`43%` de subida líquida para o jogador compensar no
      mouse. Inclui leve desvio horizontal aleatório (metade revertido na
      volta) e `60 ms` de micro-shake em roll/pitch.
    - **Suporte no SDK Horizon:** `ClientHost::set_local_rotation(yaw, pitch)`
      move a mira real do jogador sem salto entre frames, e `Hooks::camera_shake`
      aplica roll/pitch transitório de impacto por frame.
  - **Recuo visual da arma (`GunRenderingHandler`, janela de `200 ms`):**
    - Envelope assimétrico: subida linear explosiva nos primeiros `15–25%` do
      tempo (`weaponRecoilOffset` `0.725–0.97`) e retorno quadrático suave no
      restante, filtrado por um sistema massa-mola de 2ª ordem
      `SecondOrderDynamics(f = 0.5, z = 0.6, r = 2.65)` integrado com `dt` do
      frame.
    - Coice em `Z` (`recoilKick * 0.0625 * normal * adsFactor`) + subida em `X`
      ao redor de pivô traseiro perto do punho (`(0, 0.05*kick, 0.35*kick)`),
      reduzido em `-12.5%` agachado, mais micro-oscilação aleatória em `Y/Z`.

### 3. Transição de ADS e geometria real da mira (`AimingHandler` + auditoria do `.bbmodel`)

- **Dinâmica do TAC (`AimingHandler`):**
  - Progresso base `MAX_AIM_PROGRESS = 4` ticks (`200 ms`, ~`220–250 ms` efetivos),
    com ganho progressivo por tick (`0.8 → 1.3`, `+0.1`/tick) suavizado por
    `SecondOrderDynamics(f = 0.45, z = 0.8, r = 1.2)` — independente de FPS
    (diferente do `0.3/frame` atual).
  - **Trajetória em arco:** aplica um mergulho parabólico em `Y` no meio da
    transição (`-(0.033 - |0.5 - ads| * 0.066)` blocos, pico `-0.033` em
    `ads = 0.5`) e desfaz `5° * (1 - ads)` de inclinação `Z` do quadril.
  - **FOV e sensibilidade:** `fov = fovModifier + (1 - fovModifier) * (1 - ads)`
    (`0.67` em rifles de mira aberta como `m24`/`mk14`/`sks_tactical`, `0.80` em
    revólveres/escopetas). Reduz a sensibilidade do mouse proporcionalmente ao
    zoom — mapeável direto em `Hooks::calculate_player_turn`. Bloqueado ao
    recarregar ou correr.
- **Auditoria geométrica de `models/winchester_1873.bbmodel` / `winchester_1873.json`:**
  - Com `item_pose = (+0.56, -0.52, -0.72)` e `display.firstperson_righthand`
    (`translation = [0, 0.3, -4.75]` texels, `scale = 0.45`):
    - Grupo `Miras` está centrado em **`X = 8.139` texels** (`mira_frontal_ponto`
      `[8.019..8.259]`, topo `Y = 12.380`, `Z = -15.14`; `mira_traseira_folha`
      `[7.923..8.355]`, topo `Y = 12.444`, `Z = +5.36`).
    - Offset exato para zerar a massa de mira no centro da tela (`X = 0, Y = 0`):
      **`X = -0.5639`**, **`Y = +0.3775`**.
    - **Conflito visual detectado no modelo:** `cao_ponta` (`[298]`, espora do
      cão em `Z = 15.17`) sobe até **`Y = 12.644`** sobre o mesmo `X = 8.139`
      — `0.20..0.26` texels (`~0.007` blocos) **acima** da linha das miras. Se
      `Y = +0.3775`, a ponta do cão cruza a linha de visada; usar `Y ≈ +0.369`
      (visada rasante sobre o cão) ou rebaixar levemente o cão resolve.
    - Coronha (`coronha_01`, `Z = 31.614`) fica em `Z = -0.353` no quadril
      (`-0.303` com `Z = +0.05`), longe do near-plane (`-0.05`) mesmo com coice.

### 4. Máquina de estados de recarga (`ReloadHandler` / `ReloadTracker`)

- Na v0.7.1, cada aperto de `R` insere **só 1 cartucho** (`6` ticks). No TAC,
  armas de tubo (`m870_classic` e equivalentes) entram num **estado contínuo de
  recarga** com um único `R`:
  1. **Abertura (`PreReload`, `8–12` ticks):** inclina a arma expondo a janela
     lateral (`loading gate`) e abaixa o ADS.
  2. **Ciclo por cartucho (`InterReload`, `10–15` ticks/cartucho):** servidor
     valida inventário, consome 1 munição, incrementa o tubo e toca o som de
     inserção; repete sozinho até encher (`8/8`) ou acabar a munição.
  3. **Fechamento (`PostReload`, `6–8` ticks):** retorna à pose neutra.
- **Regras de interrupção:** clicar para atirar com ≥1 cartucho no tubo cancela
  a recarga na hora e permite disparar; trocar de slot na hotbar, largar o item
  ou correr também cancelam o estado no servidor e no cliente.
- **Detalhe importante no Horizon:** alterar `stack.damage` a cada cartucho/tiro
  pode disparar a animação vanilla de re-equipar a mão; implementar
  `ItemPart::should_cause_reequip_animation` retornando `false` quando o item
  continua sendo `winchester_1873` elimina esse salto visual.

### 5. Números de balanceamento (comparativo TAC vs v0.7.1)

Nota: `data/tac/guns/m1873.json` no jar é um JSON legado do revólver **Colt SAA
M1873** de 6 tiros (`damage = 8.25`, `speed = 7.5`), útil como referência futura
para o Colt SAA. Para carabina/rifle de ação manual e tubo, os parâmetros ativos
da 0.3.10.1 são:

| Arma / Ref | Dano | Velocidade (b/t) | Spread ADS | Mult. Hipfire | Recuo Cam (°) | Kick Z | FOV ADS | Cadência / Recarga |
|---|---|---|---|---|---|---|---|---|
| **Blackpowder v0.7.1** | `12.0` | `6.0` | `0.35` (fixo) | `1.0x` (igual!) | `0°` (só visual `7°`) | `0.10` bl | `0.60` | `8t` (`150 RPM`) / `6t` avulso |
| **TAC `sks_tactical`** | `10.0` | `27.5` | `0.065°` | `8.5x` | `6.25°` (`-30%` ADS) | `1.50` | `0.67` | `300 RPM` / pente |
| **TAC `mk14`** | `16.0` | `31.5` | `0.030°` | `8.5x` | `6.88°` (`-25%` ADS) | `1.63` | `0.67` | `300 RPM` / pente |
| **TAC `m24` (ferrolho)** | `21.5` | `35.5` | `0.015°` | `14.5x` | `5.75°` (`-27.5%` ADS) | `1.63` | `0.67` | `50 RPM` / ferrolho |
| **TAC `colt_python`** | `12.5` | `17.0` | `0.125°` | `4.5x` | `8.50°` (`-25%` ADS) | `2.35` | `0.80` | `133 RPM` (`9t`) |
| **TAC `m870_classic`** | `22.0` | `15.0` |pellet | — | `9.50°` (`-15%` ADS) | `1.35` | `0.80` | tubo: `12t` abre + `15t`/cartucho |

Principais discrepâncias da v0.7.1 reveladas pela tabela:
- **Velocidade `6.0` b/t** é velocidade de bola de neve forte (cai muito cedo
  com a gravidade do template `snowball`); subir para **`15.0` b/t** (`300 m/s`)
  dá balística de carabina `.44-40` mantendo leve queda a longa distância.
- **`inaccuracy = 0.35` fixo** faz o ADS ser só cosmético; separar **ADS
  (`0.06`, ou `0.04` agachado)** de **Hipfire (`1.5`, ou `2.2` em movimento)**
  torna mirar realmente recompensador.
- **Cadência e recarga:** `LEVER_TICKS = 10` (`120 RPM`) dá tempo para ler o
  recuo + ciclo da alavanca; loop de recarga com `8t` de abertura + `10t` por
  cartucho + `6t` de fechamento leva `~4.7s` de `0 → 8` com um único `R`.

## Implementado na v0.8.0

- **Opção B (cão engatilhado + mira centralizada):** elementos `cao_disco_1..3` (`[293..295]`), `cao` (`[296]`), `cao_espora` (`[297]`) e `cao_ponta` (`[298]`) rotacionados em `+62°` ao redor do pino `[0.0, 16.6, 65.6]` em `models/winchester_1873.bbmodel` e reconvertidos via `tools/bbmodel_convert.py`, baixando o topo do cão para `12.360` (abaixo da alça `12.544` e da massa `12.480`) com `ADS_OFFSET = [-0.5639, 0.3695, 0.05]`.
- **Braços duplos em 1ª pessoa (`draw_arms`):** `right_arm` + `left_arm` (com mangas) extraídos com `PartPose::ZERO`, `scale([sxz, -sy, -sxz])` e matriz de pose da arma replicada em `event.draw`, incluindo movimento de alavanca na mão direita e inserção na janela `porta_B` pela mão esquerda.
- **ADS & Recuo em 2 camadas (`render_frame_pre` + `compute_fov` + `calculate_player_turn` + `camera_pose`):** dinâmica de 2ª ordem por frame (`event.elapsed`), zoom `0.67`, sensibilidade escalada, subida de câmera com recuperação suave de `57%` (`set_local_rotation`), micro-jolt e coice visual pivotado na coronha.
- **Recarga contínua & balística (`level_tick_post` + `ToClient<RifleSync>`):** ciclo `Opening 8t -> Inserting 10t/bala -> Closing 6t` com 1 aperto de `R` (interrompível por tiro com munição no tubo, corrida ou troca de slot), `should_cause_reequip_animation` desativado para variações de `damage`, velocidade `15.0` b/t, `LEVER_TICKS = 10` e dispersão dinâmica ADS/Hipfire.

## Hotfix v0.8.1 (braços invisíveis em 1ª pessoa)

- **Causa raiz no atlas do Horizon (`native_draw_model_part`):** `self.gui.atlas.sprites` registra sprites vanilla **sem** o prefixo `"minecraft:"` (`"entity/player/wide/steve"`, igual a `"misc/vignette"` e ao `Game::first_person_arm` em `frame_geometry.rs`), enquanto `bake_layer("minecraft:player#main")` exige o prefixo. Passar `"minecraft:entity/player/wide/steve"` fazia `draw.model_part` retornar `false` sem emitir vértices.
- **Correção:** `LimbModel::draw_part_with_skin` passa `"entity/player/wide/steve"` (com fallback automático para `"minecraft:entity/player/wide/steve"`), `PlayerArms::fallback_wide` garante malha procedural `64x64` (`MeshDefinition` + `LayerDefinition`) caso `bake_layer` falhe ou `render_hand` rode antes de `client_setup`, e a inclinação/comprimento do antebraço direito foi refinada (`rx = 110°`, `sy = 0.82`) para aparecer mais no hipfire mantendo folga segura do near-plane (`z <= -0.065` mesmo no pico de recuo).

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
- O SDK é pinado (`2b77287de099c9316278ba7ac4d83590fdb7ef30` — Nightly
  2026-11-22). A API muda entre commits: confira assinaturas no SDK antes de
  inventar nomes.

## Entrega `v0.9.0` (implementada)

1. **SDK atualizado:** `Cargo.toml`, `.github/workflows/build-mod.yml` e
   `.github/workflows/release.yml` migrados para
   `2b77287de099c9316278ba7ac4d83590fdb7ef30` (`Nightly 2026-11-22`).
2. **Skin real do jogador + `slim`/`wide` + mangas 3D (`0.25` deformação):**
   - Fallback imediato no frame 0 via `default_skin(uuid)` (18 skins modernas,
     `0..9` `slim` `3x12x4` e `9..18` `wide` `4x12x4`).
   - Resolução online no servidor via `ServerHost::profile_by_uuid`, decodificando
     `textures` Base64 em `"skins/" + sha1_utf16le(hash)` + `"model" == "slim"`
     e sincronizando via canal tipado `RifleSkinSync`.
   - Ambos os braços são desenhados no espaço `[0..1]` da arma com a mesma
     proporção do glTF do TAC (`[1.0, 1.55, 1.0]` dentro da escala `0.45` =
     `[0.45, 0.70, 0.45]` no mundo), eliminando a diferença de tamanho entre os
     braços e mantendo `max_z < -0.16` longe do near plane (`-0.05`).
3. **Sub-modelos articulados + máquina de estados estilo TAC + áudio CC0 `.ogg`:**
   - `winchester_1873.json` dividido nos 6 modelos standalone em
     `assets/blackpowder_legacy/models/special/` (`body`, `lever`, `hammer`,
     `trigger`, `gate`, `cartridge`), mantendo `models/item/winchester_1873.json`
     intacto para inventário, chão e 3ª pessoa.
   - Estados completos: `Draw`, `Idle`/`Walk`/`Sprint`/`ADS`, `FireCycle`
     (gatilho, cão batendo, alavanca abrindo `+48°` e rearmando o cão),
     `DryClick`, `ReloadIntro`, `ReloadLoop` (cartucho `.44-40` entrando pela
     `porta_B` deprimida em `-23°`), `ReloadEndNorm`, `ReloadEndEmpty` (ciclo
     completo da alavanca ao recarregar seco) e `Inspect` (tecla `I`).
   - 9 sons CC0 `.ogg` empacotados em `assets/blackpowder_legacy/sounds/` +
     `sounds.json` (ver `SOUND-CREDITS.md`).

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
