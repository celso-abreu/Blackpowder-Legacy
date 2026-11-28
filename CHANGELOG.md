# Changelog — Just a Rifle

Histórico consolidado de evolução do mod (anteriormente *Blackpowder Legacy*, rebatizado para **`Just a Rifle`** / `just_a_rifle` na `v0.9.0` com foco casual em uma única arma polida nos mínimos detalhes).

---

## `v0.11.0` — Sistema de Áudio Espacial 3D (`48 kHz`), Reatividade de Mundo (TNT, Folhagens, Escudos, Velas & Sculk), Calor do Cano, Proteção de Reparo e Modularização

### Sistema Acústico 3D & Multiplayer (`src/client_audio.rs`, `src/server.rs`, `23` `.ogg` `48 kHz`)
- **Áudios 3D Mono `48 kHz` com atenuação espacial OpenAL real:** padronização de todos os **23** arquivos `.ogg` em `48 kHz` (`21` Mono espaciais + `2` Stereo locais de manuseio em 1ª pessoa), eliminando som sem atenuação em multiplayer, além de cancelamento/fade suave via `SoundRef` ao interromper inspeção, saque ou recarga.
- **Passagem balística (`crack` supersônico + `whiz` Doppler) & Eco ambiental:** projéteis que passam perto de jogadores disparam estalo supersônico N-wave (`>= 2,2` blocos/tick) ou assobio subsônico no instante exato de maior aproximação; disparos sondam luz do céu e teto para emitir reverberação de caverna/interior (`winchester_echo_cave`) ou eco aberto de vale (`winchester_echo_open`) com atraso de velocidade do som (`340 m/s`), acompanhados de concussão opcional em ambientes fechados (`audio.concussion`, **33 opções** no menu Config).
- **Impactos em corpo vs. armadura/escudo & Foley 3D remoto:** impactos em entidades distinguem tecido desprotegido (`winchester_impact_flesh`), armaduras metálicas e bloqueio de escudo (`winchester_impact_armor`), enquanto observadores em multiplayer ouvem recarga, ciclo de alavanca e cápsulas `.44-40` quicando no chão a partir da culatra de jogadores próximos.

### Interações Físicas de Mundo, Combate & Sobrevivência (`src/world_fx.rs`, `src/ballistics.rs`, `src/exploration.rs`)
- **Perfuração de folhagens (`<= 2` blocos):** balas `.44-40` atravessam copas de folhas e trepadeiras mantendo `82%` da velocidade por camada perfurada, gerando partículas de folhas cortadas sem parar no primeiro arbusto.
- **Ignição balística de TNT & Detonação de `tnt_minecart` (`14ª` Conquista):** tiros secos acendem blocos `minecraft:tnt` com pavio curto (`15` ticks) e detonam vagões de dinamite instantaneamente, desbloqueando **`Barril de Pólvora` / `Powder Keg`** (`just_a_rifle:powder_keg`).
- ** Física dupla em velas (pavio vs. cera):** tiros de raspão no pavio superior apagam a chama intacta (`Tiro de Salão` / `Candle Snuffer`), enquanto tiros em cheio no corpo ou base da vela estouram a cera em partículas coloridas da própria vela sem largar o item.
- **Impacto pesado em escudos, vibrações Sculk/Warden & Espólio de Illagers:** bloqueio em escudo aplica recuo físico e desgaste balístico (`4..10` pontos) sem falso hitmarker de headshot; disparos e impactos emitem `minecraft:projectile_shoot` e `minecraft:projectile_land` atribuídos ao atirador (respeitando `#minecraft:dampens_vibrations`); Saqueadores e outros Illagers abatidos pela Winchester deixam cair cartuchos `.44-40` escalados com *Saque* (*Looting*).
- **Aquecimento do cano & Têmpera na água:** cadência rápida aquece progressivamente o cano octogonal, gerando fios de fumaça acima da massa de mira e vapor chiando (`block.lava.extinguish`) ao mergulhar o cano quente na água.
- **Proteção de integridade da munição (`ItemStack::damage`):** bloqueio de reparo em Rebolo, combinação em bancada/inventário/Crafter, *Remendo* (*Mending*, `xp_repair_ratio = 0.0`) e reparo por material na Bigorna, preservando munição exata ao renomear a arma.

### Manutenção Estrutural, Ciclo de Vida WASM & Localização `pt_pt`
- **Modularização de `src/lib.rs` (`-1.012` linhas):** extração limpa de `src/client_audio.rs` e `src/hud.rs`, deduplicação da física de ejeção de cápsulas em `src/world_fx.rs`, passo único por tick de servidor em `tick_pending_impacts` / `tick_reload_sessions`, limpeza completa de todas as tabelas estáticas ao sair de mundos e poda periódica de jogadores/entidades desconectados.
- **Tradução `pt_pt.json` & Verificador endurecido:** suporte completo a Português Europeu (`en_us`, `pt_br` e `pt_pt` com `141` chaves sincronizadas), créditos completos dos `23` áudios em `SOUND-CREDITS.md`, remoção dos scripts legados `tools/decompile/` e novas checagens automáticas de paridade de versão/áudio/documentação em `tools/verify_assets.py`.

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
