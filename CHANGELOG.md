# Changelog — Just a Rifle

Histórico consolidado de evolução do mod (anteriormente *Blackpowder Legacy*, rebatizado para **`Just a Rifle`** / `just_a_rifle` na `v0.9.0` com foco casual em uma única arma polida nos mínimos detalhes).

---

## `v0.10.0` — Balística `.44-40` & Headshots, Poses 3D em `F5`/Multiplayer, HUD de Reserva, Tooltips Inteligentes, LOD 3D, Água, Alvos Reativos, Config (31 Opções) e Conquistas

### Balística Terminal `.44-40` & Headshots (`src/ballistics.rs`)
- **Dano base `.44-40` elevado para `14.0` (`7` corações) + Headshot `1,5×` (`21.0` = `10,5` corações):** interseção geométrica 3D sub-tick raio-AABB (`is_ray_headshot_on_box`) compensa projéteis rápidos (`15,0` blocos/tick = `300 m/s`), garantindo abate limpo em 1 disparo na cabeça em Esqueletos, Zumbis, Creepers e Aranhas (`20 HP`) mesmo a `50 m`.
- **Queda suave de dano em longo alcance (`> 40 m`):** curva Hermite (`smoothstep`) aplicada em `Hooks::living_damage_pre`, mantendo `1,00×` até `40 m` e decaindo suavemente até `0,72×` acima de `110 m`.
- **Feedback de Headshot & 13ª Conquista:** explosão cranial de partículas `minecraft:crit`, confirmação harmônica dupla no cliente (`SYNC_HEADSHOT_CONFIRM`), marcador visual duplo carmesim-dourado no HUD e conquista **`Bem Entre os Olhos` / `Between the Eyes`** (`just_a_rifle:between_the_eyes`, `goal`, `65 XP`).

### HUD de Reserva `.44-40` & Tooltips Inteligentes (`Hooks::item_tooltip`)
- **Contador de reserva com mini-ícone `.44-40`:** o painel ocidental no HUD (`8 / 8 │ [ícone .44-40] 36`) contabiliza em cache de `20 Hz` todos os cartuchos `.44-40` no inventário principal + mão secundária, exibindo **`∞`** dourado no modo Criativo (configurável em `hud.show_reserve`).
- **Tooltips dinâmicos (`en_us` / `pt_br`) sem durabilidade crua:** `Hooks::item_tooltip` oculta a linha enganosa `"item.durability"` (`92 / 100`) do `F3+H` em qualquer idioma e exibe munição no tubo + reserva, dados balísticos (`14 Dano / 21 Cabeça · 120 DPM · Queda >40m`) e **as teclas atuais remapeadas ao vivo** (`[Botão Direito]`, `[R]`, `[I]`, `[F]`) via `host.mod_key` e `TextContent::Keybind` (configurável em `hud.rich_tooltips`).

### Poses Dinâmicas de Braços em 3ª Pessoa (`F5` & Multiplayer)
- **4 poses customizadas de duas mãos (`ArmPosesRegistrar`) + sincronização `RifleThirdPersonSync`:** jogadores em `F5` e jogadores remotos no multiplayer alternam naturalmente entre porte baixo de duas mãos em repouso (`hip_ready`), guarda cruzada ao correr (`sprint_carry`), mira alinhada à cabeça `[head.xRot, head.yRot]` no ADS (`CrossbowHold`), acionamento da alavanca pós-disparo (`lever_cycle`), recarga tiro a tiro na janela lateral (`reload`) e braço livre com rifle na bandoleira das costas (`ArmPose::Empty`), com espelhamento completo para canhotos.

### Identidade do Pacote, Config de 31 Opções, Exploração, Água, Alvos Reativos & Performance LOD 3D
- **Ícone oficial `icon.png` (`256×256`) & Manifesto completo:** emblema ocidental em latão renderizado deterministicamente (`tools/make_textures.py`, `< 35 KiB`) a partir do modelo 3D real da Winchester 1873 + cartucho `.44-40`, além de `homepage`, `issues` e `credits` preenchidos em `horizon-mod.toml`.
- **Página `Options > Mods > Just a Rifle > Config` com 31 opções categorizadas:** organizada visualmente em `[Áudio]`, `[Retorno]`, `[Controles]`, `[Câmera]`, `[HUD]` e `[Mundo]`.
- **Exploração & 13 Conquistas:** injeção determinística via `Hooks::container_loot_filled` (zero conflito com outros mods) em Minas Abandonadas, Postos de Saqueadores, Mansões, Dungeons e Pirâmides, acompanhada de árvore completa com **13 conquistas** e recompensas de XP.
- **Comportamento na Água & Alvos Reativos no Mundo:** falha úmida de pólvora negra submersa (`winchester_misfire_wet`) sem gastar munição, arrasto hidrodinâmico + rastro de bolhas em projéteis, sinos badalando à distância, carrilhão musical na mosca (`15`) do `minecraft:target`, apagamento de velas acesas no tiro e estilhaçamento de vidraças, vasos e estalactites.
- **LOD 3D & Otimização WASM:** malhas dedicadas de 40 elementos (`winchester_1873_lod` e `winchester_body_lod`) eliminando travamentos de BSP na GUI/inventário, remoção lossless de faces internas ocultas (`-22%` quads em 1ª pessoa) e caches `20 Hz` na fronteira WASM.

---

## `v0.9.0` — Rebrand para *Just a Rifle*, Rig Articulado, Efeitos Físicos e Modularização

### Identidade & Arquitetura
- **Novo nome e namespace (`just_a_rifle`):** migração completa do manifesto (`horizon-mod.toml`), crate (`just-a-rifle`), pastas `assets/just_a_rifle/` e `data/just_a_rifle/`, canais de rede, aba criativa, legendas (`pt_br` / `en_us`), scripts `tools/` e pacote **`just_a_rifle.hmod`**, assumindo oficialmente a proposta casual de **arma única**.
- **Horizon SDK `f1bb66f`:** atualizado para o SDK assinado `f1bb66f353cf67f2aa4665293fc75c8d91186fc7` (`Nightly 2026-11-22`).
- **Código modularizado em `src/`:** o antigo `src/lib.rs` monolítico (~3.900 linhas) foi organizado em módulos dedicados (`constants.rs`, `skin.rs`, `arms.rs`, `pose.rs`, `world_fx.rs`, `server.rs` e `lib.rs`), acompanhados de testes unitários automatizados na CI (`cargo test --lib`) e verificação estática de assets (`tools/verify_assets.py`).

### Modelo 3D Articulado, Braços & Animações
- **8 modelos standalone (`models/special/`):** `winchester_body`, `winchester_lever`, `winchester_hammer`, `winchester_trigger`, `winchester_gate`, `winchester_cartridge`, `winchester_casing` e `bullet_hole`, todos gerados deterministicamente a partir do `models/winchester_1873.bbmodel`.
- **Animação mecânica completa:** acionamento do gatilho, queda e re-engatilhamento do cão (`HAMMER_PIVOT_TEX`), ciclo de `+48°` da alavanca (`LEVER_PIVOT_TEX`), janela lateral King's loading gate (`-23°`) recebendo o cartucho `.44-40` em 3D tiro a tiro, e sequência de inspeção na tecla **`I`** (`3.4 s`).
- **Skin real do jogador + braços `slim` (`3px`) / `wide` (`4px`):** decodificação pure-Rust da propriedade Base64 `textures` da Mojang (`UTF-16LE SHA-1`) com mangas externas 3D (`CubeDeformation(0.25)`), fallback determinístico para as 18 skins modernas do Horizon e mão esquerda envolvendo naturalmente o guarda-mão de nogueira (`guarda_mao_traseiro`).
- **Bandoleira diagonal nas costas (`F`):** ao enviar o rifle para a mão secundária (`off-hand`), a arma fica atravessada a `44°` nas costas em 3ª pessoa (`HumanoidPart::Body`), adaptando a profundidade quando o jogador usa peitoral/élitros.
- **Mira limpa em ADS:** oculta automaticamente a mira central (`minecraft:crosshair`) ao alinhar alça e massa de mira.

### Efeitos de Mundo & Áudio CC0
- **Ejeção em 2 estágios de cápsulas `.44-40` em 3D:** a cápsula deflagrada sobe primeiro pela janela superior da culatra (`trava_dorsal`, recortada pelas paredes do receptor) e faz handoff sem salto visual (`0 px`, compensado para qualquer FOV) para a física no mundo, quicando em blocos, lajes e escadas com tilintar metálico antes de repousar na horizontal.
- **Marcas de tiro no terreno (`bullet_hole`):** até `96` decalques cutout iluminados pela luz do mundo (`Block` + `Sky`) aplicados na face exata do impacto com fragmentos do bloco atingido e fumaça, removidos automaticamente se o bloco for quebrado.
- **Fumaça e clarão alinhados à `boca_cano`:** projeção exata da ponta do cano para o espaço do mundo em hipfire, ADS e `F5`.
- **9 áudios `.ogg` CC0 (Domínio Público):** disparo mono `48 kHz`, impacto grave de culatra, alavanca, cão/clique seco, recarga (abertura, inserção por cartucho e fechamento), saque/bandoleira e inspeção, todos com legendas de acessibilidade em `pt_br` e `en_us`.

---

## `v0.8.0` – `v0.8.1` — Dinâmica de 2ª Ordem, Recuo em 2 Camadas e Recarga Contínua
- **Molas de 2ª ordem independentes de FPS (`SecondOrder`):** transição suave de ADS (`FOV 0.67`), escala proporcional de sensibilidade do mouse e corrida tática (`low-ready`).
- **Recuo em 2 camadas:** coice visual com pivô na coronha + subida real da câmera com recuperação suave de `57%` e micro-tremor de disparo.
- **Recarga contínua (`R`):** um único toque em `R` alimenta o magazine tubular (`8` tiros) cartucho por cartucho, com interrupção imediata ao atirar se já houver munição carregada e sem "pulo" de re-equip (`should_cause_reequip_animation`).
- **Balística revisada:** velocidade `15.0 b/t` (`300 m/s`), dispersão real por postura (`0.04` ADS agachado / `0.06` ADS em pé vs. `1.50` no quadril / `2.20` em movimento) e cadência de `120 RPM`.

---

## `v0.5.0` – `v0.7.0` — Base Jogável no Horizon WASM
- Conversão do modelo Blockbench da **Winchester 1873** (`336` elementos, textura `1024×1024`) para o formato estendido do Horizon.
- Registro completo via `.hmod` WebAssembly: rifle, cartucho `.44-40`, projétil (`12.0` de dano), receitas de bancada, aba criativa, barra de munição dourada no ícone, contador HUD e sincronização cliente-servidor.
