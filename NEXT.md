# Próximos Passos — Just a Rifle

Roadmap ordenado por prioridade mantendo a filosofia central do **`Just a Rifle`**: **apenas uma arma** (a Winchester 1873), leve, bem integrada ao Minecraft vanilla no Horizon e polida nos mínimos detalhes.

---

## Concluído recentemente

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

3. **Mundo Reativo ao Tiro + Sons de Impacto por Material**
   - Reações físicas em blocos Vanilla ao impacto da bala: badalar Sinos (`bell`) à distância, ativar Blocos-Alvo (`target`) de Redstone conforme a precisão e estilhaçar Vasos Decorados (`decorated_pot`) / vidraças.
   - Sons de impacto específicos por material (madeira, metal/cobre, vidro, terra/areia, pedra).

4. **Poses Dinâmicas de Braço em 3ª Pessoa (`F5` / Multiplayer)**
   - Ajustar os dois braços do personagem em 3ª pessoa para diferenciar empunhadura no quadril/corrida e mira erguida em ADS.

5. **Exploração Vanilla (Loot em Estruturas + Conquistas)**
   - Chance balanceada de encontrar cartuchos `.44-40` (e raramente uma Winchester descarregada) em Minas Abandonadas, Postos de Saqueadores e Mansões, além de conquistas temáticas (`pt_br` / `en_us`).

6. **Configurações do Mod no Menu do Horizon (`mod_settings`)**
   - Opções leves no menu de Mods (alternar HUD de munição, intensidade da vinheta do ADS, quantidade/tempo das cápsulas e marcas de tiro no chão).

7. **Fabricação Realista de Armeiro & Recarga de Estojos `.44-40` (`v1.0.0`)**
   - Metalurgia de aço, fabricação das partes reais da Winchester 1873 (cano octogonal, mecanismo *toggle-link*, coronha de nogueira encerada), bancada de armeiro tátil em 3D e reaproveitamento das cápsulas de latão deflagradas com espoleta, pólvora negra e projétil `.44`.
