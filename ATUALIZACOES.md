# Plano de Atualizações — Blackpowder Legacy (`v0.8.0`)

Documento visual para revisão e aprovação do escopo da **v0.8.0**, baseado na
análise do **TAC 0.3.10.1** cruzada com o código do **Horizon SDK** (`0a875da9`).

> **Status:** Implementado na **v0.8.0** (**Opção B** aprovada — cão engatilhado no `.bbmodel`/`.json` liberando a linha de visada centralizada, ambos os braços em 1ª pessoa presos à pose da arma, ADS/recuo em 2 camadas independentes de FPS, recarga contínua por cartucho e balística Winchester 1873 calibrada).

---

## Visão geral: Antes (`v0.7.1`) vs Proposto (`v0.8.0`)

| Sistema | Como está na `v0.7.1` | Como ficará na `v0.8.0` |
|---|---|---|
| **Mãos em 1ª pessoa** | Desenha a árvore inteira do Steve sem extrair o braço, invertida no eixo Y e **congelada no quadril** (não acompanha ADS nem recuo). | **Duas mãos** extraídas (`right_arm` + `left_arm` com mangas), eixo Y corrigido e **presas à arma**: mão direita no punho, esquerda sob o guarda-mão de madeira (deslizando na recarga). |
| **Mirar (ADS)** | Lerp fixo `0.3/frame` (muda de velocidade conforme o FPS); FOV `0.60`; mesma sensibilidade de mouse; permite mirar correndo/recarregando. | Transição suave independente de FPS (`~220 ms`, mola de 2ª ordem) com leve arco natural; FOV `0.67`; **sensibilidade do mouse reduzida proporcionalmente ao zoom**; bloqueado ao correr ou recarregar. |
| **Recuo (Recoil)** | Só visual no modelo (`exp(-6p)` simples); a câmera do jogador não sobe nada. | **2 camadas:** coice explosivo + mola no modelo (pivô na coronha) **+** subida real da câmera (`~5.5°`, `-30%` mirando) com recuperação parcial (`57%`) e micro-tremor de impacto (`60 ms`). |
| **Recarga (`R`)** | Apertar `R` coloca **só 1 cartucho** e para (precisa apertar `R` 8 vezes). Troca de `damage` pode causar "pulo" de re-equip na mão. | **1 aperto de `R` recarrega continuamente** cartucho por cartucho até encher (`8/8`) ou acabar munição. **Clique de tiro interrompe na hora** se já houver bala no tubo. Sem pulo visual de re-equip. |
| **Balística e Cadência** | Velocidade `6.0` b/t (cai rápido como bola de neve); imprecisão fixa `0.35` (mirar não melhora a precisão real!); alavanca em `8` ticks (`150 RPM`). | Velocidade **`15.0` b/t** (`300 m/s`); tiro **muito preciso mirando** (`0.06`, ou `0.04` agachado) vs **espalhado no quadril** (`1.5`, ou `2.2` andando); alavanca em **`10` ticks** (`120 RPM`); dano `12.0` mantido. |

---

## Detalhamento por área

### 1. Correção dos braços em 1ª pessoa
Ao inspecionar o motor do Horizon (`frame_geometry.rs` e `host/draw.rs`), encontramos **3 bugs reais** no `draw_forearm` atual:
1. **`bake_layer("minecraft:player#main")` retorna o corpo inteiro:** faltava chamar `.get_child("right_arm")` / `"right_sleeve"` e zerar o deslocamento do ombro.
2. **`event.draw` é independente de `event.render`:** no Horizon, `draw` não herda automaticamente `render.translation` nem `render.rotation`; por isso o braço não seguia a arma.
3. **Eixo Y invertido:** modelos de entidade usam `+Y` para baixo, exigindo `draw.scale([1.0, -1.0, -1.0])`.

**O que entra na `v0.8.0`:**
- Extrair `right_arm` + `right_sleeve` (punho/gatilho) e `left_arm` + `left_sleeve` (apoio sob `guarda_mao_traseiro`).
- Aplicar a mesma matriz de pose da arma em `event.draw` para que ambas as mãos acompanhem ADS, recuo, alavanca e recarga.
- *(Limitação atual do SDK Horizon WASM: a skin customizada do jogador ainda não é exposta para mods WASM no commit pinado, então a textura permanece a padrão do Steve.)*

---

### 2. Explicação da escolha **A vs B** (o cão da arma na frente da mira)

Medindo peça por peça o nosso modelo 3D (`models/winchester_1873.bbmodel`), descobrimos que o **cão da arma** (`cao_ponta`, a peça que bate na espoleta) foi modelado levantado (desengatilhado) e é **mais alto** do que as miras da arma, exatamente na mesma linha central:

```text
Visão lateral quando o jogador encosta o olho para mirar:

                        (12.64 — MAIS ALTO!)
                           █   <- Ponta do cão (bem na frente do olho)
  (Olho) ───────────────►  █ ──────►  ▄ (12.44 Alça) ──────►  ▄ (12.38 Massa frontal)
  ==================================================================================
                          Culatra           Meio do cano             Ponta do cano
```

Na prática: se alinharmos a mira exatamente na altura do olho, **a ponta do cão fica bem na frente da câmera tampando a mira**. Existem dois jeitos simples de resolver:

- **Opção A — Só no código (sem mexer no modelo 3D):**
  Subimos a linha do olho um fiozinho (`Y = +0.369` em vez de `+0.3775`) para o jogador enxergar **por cima** da ponta do cão.
  - *Prós:* Não toca no `.bbmodel` nem no `.json` do modelo.
  - *Contras:* A mira fica levemente abaixo do centro exato da tela.

- **Opção B — Engatilhar/baixar o cão no modelo 3D (mais fiel à arma real):**
  Numa Winchester 1873 pronta para atirar, o cão fica **puxado para trás/baixo (engatilhado)**, deixando o caminho livre para mirar. Inclinamos levemente o grupo do cão no `.bbmodel` (e regeramos `winchester_1873.json`) para ele ficar abaixo de `12.30`, e cravamos a mira exatamente no centro da tela (`Y = +0.3775`, `X = -0.5639`).
  - *Prós:* Linha de mira limpa e cravada no centro matemático da tela, fiel à arma engatilhada.
  - *Contras:* Atualiza `models/winchester_1873.bbmodel` e `assets/.../winchester_1873.json`.

---

### 3. Curva de recuo estilo TAC (`SecondOrderDynamics`)
- **Modelo 3D (`200 ms`):** coice rápido para trás em `Z` e giro em `X` com pivô na coronha, amortecido por sistema massa-mola (`f = 0.5, z = 0.6, r = 2.65`), com `-27.5%` mirando e `-12.5%` agachado.
- **Câmera real:** sobe nos primeiros `100 ms`, segura `100 ms` e recupera `57%` da subida sozinha (`set_local_rotation`), deixando uma leve compensação manual no mouse + micro-shake de `60 ms` (`camera_shake`).

---

### 4. Máquina de estados de recarga contínua
Fluxo ao apertar **`R` uma única vez**:
1. **Abrir (`8 ticks` / `0.4s`):** abaixa o ADS e inclina a carabina mostrando a janela lateral (`loading gate`).
2. **Inserir (`10 ticks` / `0.5s` por cartucho):** consome 1 `.44-40`, soma 1 no tubo, toca o clique metálico e anima a mão esquerda empurrando o cartucho; repete sozinho até `8/8` ou acabar munição (`~4.7s` recarga completa).
3. **Fechar (`6 ticks` / `0.3s`):** volta à posição de tiro.
- **Cancelamentos instantâneos:** clicar para atirar (com ≥1 bala no tubo), trocar de slot na hotbar, largar a arma ou correr.
- **`should_cause_reequip_animation = false`:** impede o Minecraft/Horizon de baixar a mão toda vez que a barra de munição atualiza.

---

### 5. Tabela de números de balanceamento (`Winchester 1873`)

| Parâmetro | `v0.7.1` | `v0.8.0` (proposto) | Motivo / Referência TAC |
|---|---|---|---|
| **Dano** | `12.0` (6 corações) | **`12.0`** (mantido) | Mata mobs comuns (`20 HP`) em 2 tiros; alinhado a `sks` (`10`) / `colt_python` (`12.5`). |
| **Capacidade do tubo** | `8` | **`8`** (mantido) | Fiel à carabina curta `.44-40`. |
| **Velocidade da bala** | `6.0` b/t | **`15.0` b/t** | `6.0` cai como bola de neve; no TAC armas variam de `15` a `35` b/t. |
| **Imprecisão em ADS** | `0.35` | **`0.06`** (`0.04` agachado) | Faz mirar na alça/massa realmente acertar onde aponta. |
| **Imprecisão Hipfire** | `0.35` | **`1.50`** (`2.20` andando) | Penaliza tiro sem mirar a média/longa distância (padrão TAC `8x–14x`). |
| **Ciclo da alavanca** | `8` ticks (`0.4s`) | **`10` ticks (`0.5s`)** | `120 RPM`: dá tempo de sentir o coice antes de ciclar a alavanca. |
| **FOV em ADS** | `0.60` (`-40%`) | **`0.67` (`-33%`)** | Padrão exato de rifles de mira metálica aberta no TAC (`m24`, `mk14`, `sks`). |

---

## Checklist rápido para aprovação

Para liberar a implementação da **v0.8.0**, basta confirmar:

1. **Escopo geral (itens 1 a 5):** Aprovado ou quer tirar/ajustar algum número?
2. **Cão na frente da mira:** Prefere **Opção A** (olhar um fio por cima, sem tocar no modelo 3D) ou **Opção B** (baixar/engatilhar o cão no modelo 3D e cravar a mira no centro)?
3. **Mira cruzada (`+`) do Minecraft ao mirar (ADS):** Manter visível ou ocultar quando a arma estiver totalmente alinhada no olho?
