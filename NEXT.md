# Próximos Passos — Just a Rifle

Roadmap ordenado por prioridade mantendo a filosofia central do **`Just a Rifle`**: **apenas uma arma** (a Winchester 1873), leve, bem integrada ao Minecraft vanilla no Horizon e polida nos mínimos detalhes.

---

## Concluído recentemente

- [x] **Exploração Vanilla (Loot em Estruturas + Árvore de Conquistas)**
  - **Loot balanceado em estruturas (`Hooks::container_loot_filled`):** Cartuchos `.44-40` e chance rara de encontrar uma Winchester 1873 descarregada (`0 / 8`, acompanhada de alguns cartuchos avulsos) em **Minas Abandonadas** (incluindo *chest minecarts*), **Postos de Saqueadores** e **Mansões da Floresta**, além de pequenos lotes ocasionais de munição em Masmorras subterrâneas e Pirâmides do Deserto.
  - **12 Conquistas (`data/just_a_rifle/advancement/` em `pt_br` e `en_us`):**
    - Desbloqueio automático das receitas de `.44-40` e `Winchester 1873` no Livro de Receitas ao obter pólvora, cobre, ferro ou munição.
    - Aba própria **`Just a Rifle`** com **10 conquistas visíveis** (incluindo conquista secreta): *A Arma que Conquistou os Blocos*, *Relíquia de Prospector* (achar rifle em estrutura), *Oito no Tubo* (`8 / 8`), *Mantenha a Pólvora Seca* (secreta — falha submersa), *Hora do Duelo* (sino a `30+` blocos), *Tiro de Salão* (apagar vela acesa no tiro), *Olho de Águia* (mosca `15` no Bloco-Alvo a `50+` blocos), *Lei da Fronteira* (derrotar Illager) e *Atirador de Elite* (abater monstro hostil a `60+` blocos).

- [x] **Página de Configuração Completa (`Options > Mods > Just a Rifle > Config`)**
  - **29 opções ao vivo** (localizadas em `pt_br` e `en_us`, com *tooltips* explicativos e zero impacto de FPS via snapshot em cache atualizado em `setting_changed`):
    - **Mixer de Áudio por Canal (`0–200%`) + Timbre:** Volume Geral, Disparos, Grave do Tiro (*sub-bass*), Timbre do Disparo (`Original` / `Grave` / `Seco`), Alavanca e Cão, Recarga, Saque e Inspeção, Cápsulas no Chão e Impactos no Mundo.
    - **Retorno Sonoro & Tático:** Volume de Retorno Sonoro dedicado, som privado de Confirmação de Acerto em Alvo (`Sutil` / `Sino` / `Estalo` / `Off`), modo do Carrilhão de Bloco-Alvo (`Centro 12+` / `Só Mosca 15` / `Off`) e Alerta de Clique Seco.
    - **Controles, Câmera, HUD & Efeitos no Mundo:** Modo de Mira (`Segurar` / `Alternar`), Recarga Automática, Sensibilidade/Zoom/Recuo/Balanço, Posição e Cor RGB do HUD/Barra, Vinheta, Mira `+`, Hitmarker (`×`), limites/duração de Cápsulas e Marcas de Tiro, e estilo da Bandoleira nas Costas.

- [x] **Mundo Reativo ao Tiro + Sons de Impacto por Material**
  - Sinos (`minecraft:bell`) badalam em 3D de qualquer ângulo, Blocos-Alvo (`minecraft:target`) ganham carrilhão de precisão (`12+` / `15`), vidraças/vasos/estalactites estilhaçam respeitando gamerules e Modo Aventura, velas acesas apagam no tiro, e cada família de material tem áudio/partículas próprios.

- [x] **Comportamento realista na água (Pólvora Negra vs. Submersão)**
  - Falha de ignição submersa (*clique úmido* sem gastar cartucho), cano na lâmina d'água com jato de bolhas, splash + cavitação + arrasto hidrodinâmico (~3–3,5m) e cápsulas afundando suavemente.

---

## Fila de Prioridades (`v0.10.0` → `v1.0.0`)

1. **Munição Reserva `.44-40` no HUD + Tooltips Inteligentes (`pt_br` / `en_us`)**
   - Exibir a contagem de cartuchos na mochila ao lado do `8 / 8` no HUD (com ícone mini do `.44-40` e `∞` no Criativo).
   - Adicionar tooltips informativos (balas no tubo, dano, teclas dinâmicas atuais `[R]`, `[I]`, `[F]`) e ocultar a falsa linha de *"Durabilidade"* do `F3+H`.

2. **Balística Recompensadora (Headshot `1.5×` + Queda Suave a Longa Distância)**
   - Multiplicador de `1.5×` para disparos na cabeça com feedback sutil de acerto crítico e leve perda de energia acima de ~40 blocos.

3. **Poses Dinâmicas de Braço em 3ª Pessoa (`F5` / Multiplayer)**
   - Ajustar os dois braços do personagem em 3ª pessoa para diferenciar empunhadura no quadril/corrida e mira erguida em ADS.

4. **Fabricação Realista de Armeiro & Recarga de Estojos `.44-40` (`v1.0.0`)**
   - Metalurgia de aço, fabricação das partes reais da Winchester 1873 (cano octogonal, mecanismo *toggle-link*, coronha de nogueira encerada), bancada de armeiro tátil em 3D e reaproveitamento das cápsulas de latão deflagradas com espoleta, pólvora negra e projétil `.44`.
