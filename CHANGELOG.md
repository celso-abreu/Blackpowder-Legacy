# Changelog — Just a Rifle

Histórico consolidado de evolução do mod (anteriormente *Blackpowder Legacy*, rebatizado para **`Just a Rifle`** / `just_a_rifle` na `v0.9.0` com foco casual em uma única arma polida nos mínimos detalhes).

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
