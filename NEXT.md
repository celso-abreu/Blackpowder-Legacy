# Próximos Passos — Just a Rifle

Roadmap ordenado por prioridade mantendo a filosofia central do **`Just a Rifle`**: **apenas uma arma** (a Winchester 1873), leve, bem integrada ao Minecraft vanilla no Horizon e polida nos mínimos detalhes.

---

## Concluído recentemente

- [x] **Mundo Reativo ao Tiro + Sons de Impacto por Material**
  - Sinos (`minecraft:bell`) badalam e balançam em 3D de qualquer distância e ângulo (inclusive tiros de baixo para cima em torres de vila), com faíscas metálicas.
  - Blocos-Alvo (`minecraft:target`) mantêm o pulso nativo de Redstone (`1..=15`) e conquistas, ganhando retorno sonoro cristalino de precisão em acertos no centro (`power >= 12` e *bullseye* `15`).
  - Vidraças (`glass_pane` / `*_stained_glass_pane`), Vasos de Flores (`#minecraft:flower_pots`) e Estalactites (`pointed_dripstone`) estilhaçam ao impacto do `.44-40` (ao lado de `decorated_pot` e `chorus_flower`), respeitando `projectiles_can_break_blocks`, `block_drops` e Modo Aventura, além de apagar velas acesas (`#minecraft:candles`) sem quebrá-las.
  - Áudio e partículas de impacto específicos por família de material via `BlockSoundType` (`Metal` com ricochete e faíscas, `Crystal`/Ametista com carrilhão, `Glass`, `Wood`, `Earth`, `Soft` e `Stone`), e filtro geométrico que impede decalques de tiro flutuando no ar em cercas, barras, correntes, sinos ou blocos estilhaçados.

- [x] **Comportamento realista na água (Pólvora Negra vs. Submersão)**
  - Falha de ignição submersa (*clique úmido* + bolhas na culatra/cano sem desperdiçar o cartucho `.44-40`, bloqueando também recarga debaixo d'água).
  - Tiro com a ponta do cano (`boca_cano`) tocando a lâmina d'água substitui fogo/fumaça seca por jato pressurizado de bolhas + spray.
  - Entrada da bala na água gera splash na superfície, trilha de cavitação (`minecraft:bubble`) e forte arrasto hidrodinâmico (~3–3,5 blocos de alcance útil subaquático).
  - Impactos no fundo do rio/mar abafados com bolhas no lugar de poeira/fumaça seca, e cápsulas `.44-40` de latão afundando suavemente com som abafado.

---

## Fila de Prioridades (`v0.10.0` → `v1.0.0`)

1. **Munição Reserva `.44-40` no HUD + Tooltips Inteligentes (`pt_br` / `en_us`)**
   - Exibir a contagem de cartuchos na mochila ao lado do `8 / 8` no HUD (com ícone mini do `.44-40` e `∞` no Criativo).
   - Adicionar tooltips informativos (balas no tubo, dano, teclas dinâmicas atuais `[R]`, `[I]`, `[F]`) e ocultar a falsa linha de *"Durabilidade"* do `F3+H`.

2. **Balística Recompensadora (Headshot `1.5×` + Queda Suave a Longa Distância)**
   - Multiplicador de `1.5×` para disparos na cabeça com feedback sutil de acerto crítico e leve perda de energia acima de ~40 blocos.

3. **Poses Dinâmicas de Braço em 3ª Pessoa (`F5` / Multiplayer)**
   - Ajustar os dois braços do personagem em 3ª pessoa para diferenciar empunhadura no quadril/corrida e mira erguida em ADS.

4. **Exploração Vanilla (Loot em Estruturas + Conquistas)**
   - Chance balanceada de encontrar cartuchos `.44-40` (e raramente uma Winchester descarregada) em Minas Abandonadas, Postos de Saqueadores e Mansões, além de conquistas temáticas (`pt_br` / `en_us`).

5. **Configurações do Mod no Menu do Horizon (`mod_settings`)**
   - Opções leves no menu de Mods (alternar HUD de munição, intensidade da vinheta do ADS, quantidade/tempo das cápsulas e marcas de tiro no chão).

6. **Fabricação Realista de Armeiro & Recarga de Estojos `.44-40` (`v1.0.0`)**
   - Metalurgia de aço, fabricação das partes reais da Winchester 1873 (cano octogonal, mecanismo *toggle-link*, coronha de nogueira encerada), bancada de armeiro tátil em 3D e reaproveitamento das cápsulas de latão deflagradas com espoleta, pólvora negra e projétil `.44`.
