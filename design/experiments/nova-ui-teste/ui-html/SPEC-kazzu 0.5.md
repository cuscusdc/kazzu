# kazzu — Especificação para implementação em Rust + Slint (Android)

**Versão desta spec:** 1.6 (Início reorganizado, toca-discos 3D, player com disco de vinil; sobre a v1.5)
**Plataforma-alvo:** Android (exclusivo nesta fase). Desktop não está no escopo.
**Referência visual:** `kazzu0_4.html` (mesma pasta). Quando houver dúvida de layout, cor, medida
ou comportamento, o HTML é a fonte da verdade. Esta spec explica o HTML e define o que
precisa ser reproduzido em Slint, o que é aproximado e o que exige renderização especial.

Este documento não descreve o backend. Ele não implementa rede, conta, Last.fm, Discord,
download nem reprodução real; essas partes aparecem como ações simuladas (seção 9).

---

## 0. Histórico desta revisão (v1.6)

Mudanças sobre a v1.5 (a identidade visual e os tokens não mudaram):

- **Início reorganizado.** Chips no topo, **playlists em cima** (grade compacta de 2 colunas), **músicas embaixo**
  (Escolha rápida em grade fixa 3×3) e **Artistas no fim**, com seta de troca de conjunto (seção 3.2).
  A seção Artistas saiu da Biblioteca (seção 3.4).
- **Toca-discos 3D no Explorar.** Base com espessura, prato cromado com estroboscópio, tapete, pino, braço com
  contrapeso e cabeça, suporte do braço, LED, botão liga/desliga e pitch decorativo (seção 7.5).
- **Player com disco de vinil.** Três visuais: **Só disco** (padrão, com agulha), **Capa + disco** (disco saindo
  de trás da capa) e **Só capa** (seção 7.8). Escolha pelo menu **Mais** (`...`, seção 3.13).
- **Agulha animada.** Desce e sobe com easing de 1,4 s; tocando, avança em linha contínua da borda ao centro
  conforme o progresso da faixa.
- **Giro visível e independente de "Animações expressivas".** O disco do player e o do toca-discos giram mesmo
  com o toggle desligado (seção 7.7). As marcas assimétricas do disco existem porque capas simétricas escondiam o giro.
- **Ilha de controles mais baixa.** De ≈ 278 para ≈ 205 dp (−26%) em 390×780, mantendo os mesmos blocos:
  menos padding e espaço, botões e waveform menores, e a saída de áudio virou o 5º botão da linha de ferramentas.
  A fórmula de `--cover` foi reajustada (`100cqh − 366 px`; `− 330 px` abaixo de 760 dp; `− 308 px` abaixo de 660 dp).
- **Seletor de minutagem alinhado.** O marcador da waveform agora é sempre centralizado na altura da waveform,
  em qualquer altura de tela (seção 10.6). A waveform continua dentro da ilha de controles.

## 0.1 Histórico da revisão anterior (v1.5)

Esta revisão manteve a identidade (Liquid Glass + Material 3 Expressive, tema escuro padrão,
dock e bolha de pesquisa, capas geradas como placeholder). Não houve reformulação do zero.
Mudanças principais:

- **Player sem rolagem.** Antes, o player rolava quando a tela era baixa (em 360×640 estourava
  77 px). Agora ele não rola em nenhuma altura testada; a capa se ajusta à altura livre (seção 7.2).
- **Capa maior e hierarquia melhor.** A capa passou de 290 px fixos para 201–361 px, conforme a
  tela. Título 27 px (24 px em telas baixas).
- **Letra imersiva.** Linha ativa centralizada e em destaque, linhas distantes desfocadas e
  menores, sem rolagem manual. Toque em uma linha pula para ela quando há tempos (seção 7.4).
- **Vidro configurável.** Aparência ganhou o grupo "Vidro e textura" com cinco controles:
  intensidade, desfoque, transparência, brilho e granulação (seção 5.3).
- **Refração e reflexos.** Realce de borda interno mais forte, com o raio controlado pela
  intensidade do vidro. A refração de verdade (deslocamento do fundo) não é possível em CSS; ver §11.
- **Contraste corrigido nos dois temas.** Quatro problemas reais: o botão de pesquisa era
  invisível no claro; o acento do tema claro ficava com 3,3–3,7:1 em capas amarelas; cards de
  estatística e toast tinham cores fixas; o rodapé de Configurações estava com 3,55:1 em texto.
  Medidas na seção 2.2.
- **Desempenho reduzido de verdade.** O modo "Economia de bateria" agora remove vidro desfocado,
  malha animada, grão e reflexos. Aparelhos com até 4 núcleos ou 3 GB de RAM começam nesse modo.
- **Animações revisadas.** Removido o "respirar" da capa que repintava um elemento com desfoque
  de 34 px a cada quadro; o magnetismo ficou só no play/anterior/próxima e na bolha de pesquisa.
- **Áreas seguras e tamanhos.** Medidas em `dvh`/`cqh`, insets do sistema como variáveis
  (`--safe-t`, `--safe-b`) e alça de arrasto na fila.
- **Correções de consistência.** Botões de voltar e avatares com o mesmo tamanho em todas as
  telas; toast e fila usam tokens; filtro de Configurações esconde grupos vazios.
- **Spec corrigida.** O mini player tem **64 px** (o HTML usa `--island-h: 64px`). A versão anterior
  da spec dizia 58 px, o que estava errado.

---

## 1. Visão geral

- App de música em formato de celular. Referência: **390 × 780 dp**, fundo `#0b0a11`.
- Faixa de validação obrigatória: **320×568 a 600×960 dp** (ver tabela da seção 7.2).
- Navegação por **pilha de telas** com botão voltar. Três abas no dock (Início, Explorar, Biblioteca) mais uma bolha de pesquisa. O Replay abre pelo
  **ícone de gráfico** ao lado do avatar no cabeçalho.
- **Mini player** fica sempre acima do dock. Tocar nele abre o **player expandido** (tela cheia).
- Tema escuro por padrão, com variante clara. Identidade Apple-like de vidro, profundidade e
  movimento contido, com a expressividade do Material 3.
- Gradientes, brilhos e grão existem **somente na camada visual FX (seção 10)**. Dados e controles
  (preenchimento de slider, cor de texto) não usam degradê.
- Dados de exemplo: `TRACKS`, `CATEGORIES`, `LIBRARY`, `REPLAY`, `SETTINGS_PAGES` no HTML.

---

## 2. Tokens de design

### 2.1 Cores (Material 3 You)

O app não tem paleta fixa. As cores são calculadas a partir de uma **cor semente**:
- Padrão: a cor da capa da faixa atual (`Track.art`). Com o toggle **Cor da capa (You)** desligado,
  a semente é `#6750A4`.
- Tema escuro (padrão) ou claro: toggle **Tema escuro**.
- A cada troca de faixa, o esquema é recalculado (`applyScheme`).

| Papel M3 | Alias no app | Uso |
|---|---|---|
| surface | `bg` | fundo do app |
| surfaceContainerLow | `bg-player` | fundo do player (tema claro) |
| surfaceContainer | `card` | grupos, cartões, campo de busca, painel de vidro |
| surfaceContainerHigh | `card-hover` | estado pressionado, base do vidro |
| primary | `accent` | toggles ligados, avatar, chips ativos, play, valores de slider |
| onPrimary | `on-accent` | texto e ícone sobre `accent` |
| tertiary | `accent-2` | destaque secundário (faixa tocando, modos ligados) |
| primaryContainer | `accent-container` | destaque suave (faixa atual na fila, botão Letra ativo) |
| onSurface | `text` | texto principal |
| onSurfaceVariant | `text-2` | legendas e textos secundários |
| outline | `line` (divisórias), `text-3` (só ícones) | divisórias e chevrons |

**Tons usados.**
- Escuro: superfícies 6–22; primary 80; onPrimary 20; primaryContainer 30; onPrimaryContainer 90;
  tertiary 80 (matiz + 60°).
- Claro: superfícies 98–90; onPrimary branco.
- **Primary no claro não é tom fixo.** O app escolhe o tom mais claro que ainda atinge **4,8:1**
  sobre branco (`accentForLight`). Isso garante texto em `accent` com contraste AA, inclusive para
  capas com matiz amarelo, que parecem claras demais em HSL. Tertiary segue a mesma regra.

**Limite da aproximação.** O cálculo usa HSL com tons aproximados e uma correção de contraste,
não o HCT oficial. Para fidelidade total no Slint, gere o esquema com `material-color-utilities`
(ou o Material Theme Builder) a partir da semente e use o resultado como `Theme` (seção 8.1).

### 2.2 Contraste medido

Medido no navegador com as cores que o app gera. "Texto" exige 4,5:1 (WCAG AA, texto normal).
"Não texto" (ícone, borda, divisória) exige 3:1.

| Par | Uso | Escuro (semente padrão) | Claro (semente padrão) | Claro (capa Creep) |
|---|---|---|---|---|
| text / bg | texto principal | 15,2 | 16,9 | 16,7 |
| text / card | texto em cartões | 13,3 | 15,4 | 15,3 |
| text-2 / card | legendas | 10,1 | 8,1 | 7,2 |
| text-2 / bg | legendas na tela | 11,6 | 8,9 | 7,8 |
| on-accent / accent | texto em chips e play | 7,6 | 4,8 | 4,8 |
| accent / bg | destaque em texto | 10,1 | 4,6 | 4,6 |
| accent-2 / bg | faixa tocando, modos | 11,1 | 4,6 | 4,6 |
| text / tint do player | título sobre a cor da capa | 11,6 (tint `#362512`) | — | — |
| text / accent-container | faixa atual na fila | 8,7 | 13,6 | 14,4 |
| accent / card | ícones de check e toggles (não texto) | — | 4,2 | 4,2 |
| text-3 / bg | chevrons (não texto) | — | 4,3 | 3,6 |

Todos os pares usados como **texto** passam de 4,5:1 nos dois temas. Pares "não texto" passam de 3:1.
Regras para o desenvolvimento:
- Texto de valor de slider usa `text-2`, não `accent`.
- Rodapé e legendas usam `text-2`. `text-3` é só para ícones e chevrons.
- Texto sobre `accent-container` usa `text`, não `accent-2`.

### 2.3 Tipografia

- **Família:** Roboto Flex (fonte do Android; é a família do M3 Expressive). Fallback: Roboto.
  Para o eixo `wdth` da letra, ver §11 (fontes variáveis).

| Papel | Tamanho | Peso | Observação |
|---|---|---|---|
| Título grande (hero) | `clamp(34, 10,5vw, 44)` px; na barra superior `clamp(30, 9vw, 38)` | 800 | tracking −1,7 px, altura de linha 1,02 |
| Título de seção | 22 px | 800 | tracking −0,6 px |
| Título do player | 27 px (24 px em telas baixas) | 800 | tracking −0,8 px |
| Artista (player) | 14 px (13 px em telas baixas) | 600 | `text-2` |
| Letra (linha inativa) | 23 px | 700 | opacidade 26%, desfocada |
| Letra (linha ativa) | 23 px | 800 | largura `wdth` 112 |
| Texto de linha | 16 px | 500 | título de linha de ajustes |
| Título de cartão de faixa | 15 px | 700 | |
| Corpo / legenda | 13–14 px | 500–600 | |
| Rótulo caps | 12 px | 700 | tracking 1,6 px |
| Tempo / contagem | 12 px | 600 | números tabulares |
| Rótulo do dock | 11 px | 600 (ativo 700) | |

### 2.4 Formas

- Raio de cartão **28 px** e de tile **20 px** com formas expressivas. Desligado: 18 px e 14 px.
- Pílulas (`radius-pill`): 999 px.
- Squircle (cantos contínuos): raio × 1,45 em tiles, categorias, cartões, capa e letra.
  Mini player 30, dock 32, ilha de controles 40. Em Slint, ver §11.

### 2.5 Espaçamento e áreas de toque

- Margem lateral de telas: **16 px** (`--pad`).
- Espaço entre seções: 24 px antes do título, 12 px depois.
- Botões visuais: **40 px** (ícones de barra, avatar 36 px, voltar 40 px, play 60 px).
- **Área de toque mínima: 48 dp** no Android. No Slint, o `TouchArea` deve ser maior que o
  desenho (ex.: botão de 40 px com área de 48 px). No HTML, a área de toque é o próprio botão.

### 2.6 Liquid Glass (tokens)

Valores iniciais. Os valores atuais são controlados pelo painel de vidro (seção 5.3).

| Token | Padrão | Uso |
|---|---|---|
| `--gl-tint` | 53% | opacidade da base do vidro (`card-hover`) |
| `--gl-blur` | 24 px | desfoque do fundo atrás do vidro |
| `--gl-bright` | 1,07 | brilho do vidro |
| `--gl-rim` | 0,45 | força do realce de borda (refração simulada) |
| `--gl-grain` | 0,09 | opacidade da granulação do app |

---

## 3. Estrutura de telas

Telas: `home`, `explore`, `library`, `replay`, `search`, `settings`, `settings-page`, `appearance`.
Overlays: `player` (tela cheia), `queue` (gaveta de baixo), `toast`.

### 3.1 Ilha inferior (sempre visível, exceto no player e na fila)

Fica a 10 px das bordas laterais e a `10 px + área segura inferior` da base. Espaço vertical de 8 px.

- **MiniPlayer** (altura **64 px**, raio 20; squircle 30): capa 42 px · título (14 px, bold) ·
  artista (12 px, `text-2`) · botões de 40 px: **curtir** (coração; ligado = preenchido na cor
  `accent`) · **anterior** · **play/pause** · **próxima** · barra de progresso de 2 px no rodapé
  (cor `accent`). Os botões não propagam o toque; tocar no resto da área abre o player. O título
  trunca com reticências.
- **Dock** (altura 64 px, raio 22; squircle 32, vidro): 3 botões com ícone + rótulo 11 px.
  Ativo: texto `text` em negrito. A pílula de vidro é uma única peça que desliza até o item ativo
  (altura 46 px, raio 18), com animação em mola de 0,6 s. Sem pílula, fundo `rgba(255,255,255,0.16)`.
- **Bolha de pesquisa** (58 px, circular, ao lado do dock):
  - Escuro: fundo `accent` 42% sobre `rgba(20,15,25,0.35)`, ícone branco. Ativa: `accent-2` 60%.
  - Claro: fundo `accent` 16% sobre branco 62%, ícone na cor `accent`. Ativa: `accent-2` 30%, ícone `text`.

### 3.2 Início (`home`)

1. Cabeçalho: título "Início" + pílula com ícone de **gráfico** (abre o Replay) e avatar (círculo 36 px,
   `accent`, letra "C"). O ícone de histórico não aparece aqui; fica só na Biblioteca.
   A mesma pílula (gráfico + avatar) aparece em Explorar e na Biblioteca.
2. **Chips** (altura 32 px, rolagem horizontal): Tudo (ativo), Playlists, Músicas, Podcasts. Só "Tudo" tem estado;
   os outros mostram o aviso "Em breve".
3. Seção **"Suas playlists"** (+ "Ver tudo" → Biblioteca): grade de **2 colunas**, espaço de 8 px. Cada célula
   (`pl-cell`): altura 56 px, raio de tile, fundo `card` a 88%, capa quadrada de 56 px à esquerda (cor + glifo de
   `LIBRARY.playlists` e `SUGGESTED_PLAYLISTS`) e nome em até 2 linhas (14 px bold). São 6 células. Toque = aviso "Em breve".
4. Seção **"Escolha rápida"** (+ "Ver tudo" → Explorar): grade fixa **3 × 3**, todos os tiles quadrados e do
   mesmo tamanho (lado = `(largura − 52) / 3`), espaço de 10 px. São 8 músicas (`TRACKS[0..7]`) + o tile
   "Aleatório" (ícone shuffle, sem capa) na 9ª posição. Cada tile = capa cheia + rótulo em pílula escura
   (`rgba(0,0,0,0.6)`, texto **branco fixo** 12 px). **Sem bento** nesta tela.
5. Seção **"Artistas"** (fim da tela): 3 artistas por conjunto, em linha (`art-grid`, 3 colunas, espaço de 12 px).
   Cada um: capa **redonda** (círculo, lado = coluna) + nome (14 px bold) + contagem ("3 músicas"), 1 linha com
   reticências. A lista junta `LIBRARY.artists` com os artistas únicos de `TRACKS` (capa = a da primeira faixa).
   Abaixo, alinhados à direita, os botões **`>`** e (a partir do 2º conjunto) **`<`**, iguais aos de "Músicas
   sugeridas" (40 dp, mesma animação de deslizar 18 dp com fade). Cada toque **substitui** o conjunto (com poucos
   artistas a ordem recomeça). Estado: `artist_page` (≥ 0). Toque em um artista = aviso "Em breve".

O carrossel de discos foi para Explorar (seção 3.3).

### 3.3 Explorar (`explore`)

De cima para baixo:
1. Título "Explorar" + pílula (gráfico + avatar).
2. **"Sua próxima descoberta":** carrossel de **capas com vinil** + **toca-discos** (seção 7.5).
   Cada capa é um quadrado de 100 dp com o disco de 92 dp saindo 40 dp pela direita. A capa e o
   disco do centro ficam em tamanho cheio; os vizinhos, menores e mais apagados. Abaixo, título e
   artista da faixa central e o toca-discos 3D (230 × 142 dp, seção 7.5).
   - Tocar numa capa lateral centraliza. Tocar na capa central **abre**: o disco sai, voa até o
     prato, o braço desce e a faixa toca.
   - Tocar no prato alterna tocar/pausar (o braço levanta ao pausar).
   - Tocar de novo na capa central (ou numa capa diferente) devolve o disco à capa.
3. **"Músicas sugeridas":** grade fixa de **2 colunas × 3 linhas** (6 músicas), espaço de 10 × 12 dp.
   Cada célula (`sug-cell`, raio de tile, fundo `card` a 88%): capa 48 dp + título (14 px bold) e artista
   (12 px, `text-2`), ambos em 1 linha com reticências. Toque = toca.
   No fim da grade há uma **seta `>`** (botão de 40 dp): a cada toque o conjunto é **substituído** por 6
   músicas novas, **sem limite** (com poucas faixas de exemplo a ordem recomeça; com um catálogo real,
   buscar o próximo conjunto do servidor). Depois do 1º conjunto aparece também `<`, que traz o conjunto
   anterior. A troca desliza 18 dp com fade, na direção da seta (sem animação com "Animações expressivas"
   desligado). A altura da seção não muda. Estado: `suggest_page` (≥ 0).
4. **"Playlists sugeridas":** carrossel de capas quadradas (150 px), dados em `SUGGESTED_PLAYLISTS`.
5. **"Categorias":** grade **2 colunas**, altura 150 px, raio 16. **Bento:** a 1ª categoria ocupa as
   2 colunas (altura 172 px). Cada uma: título branco (16 px, extra-negrito) no topo, faixa de cor
   sólida na base (52%), miniatura de capa (44% da largura) no canto inferior direito.
   Toque = toca a faixa vinculada (`track`).

### 3.4 Biblioteca (`library`)

1. Cabeçalho (pílula com ícone de histórico + gráfico + avatar; só aqui há histórico) + título "Biblioteca". O ícone de histórico
   mostra o aviso "Em breve" até existir uma tela de histórico.
2. Seção "Continue ouvindo" (+ "Histórico" → aviso "Em breve"): carrossel horizontal, cartões 150 px,
   3 faixas. Toque = toca a faixa. Os cartões de estatística do Replay **não ficam mais aqui**;
   abrem pelo ícone de gráfico do cabeçalho (seção 3.5).
3. Grupo de linhas: Downloads, Música local, Nova playlist (ícone + título + chevron).
4. Seção "Playlists" (+ "Ver tudo"): carrossel de capas quadradas (150 px).

Os artistas **não aparecem mais na Biblioteca**: estão no fim do Início (seção 3.2).

### 3.5 Replay (`replay`)

Abre pelo ícone de gráfico ao lado do avatar. Não é aba do dock.

- Botão voltar (40 px, círculo com 10% de `text`) + avatar.
- Título "Replay" + subtítulo com o período selecionado (Esta semana / Este mês / Este ano / Todo o tempo).
- **Segmentado** (fundo `card`, raio 14): Semanal · Mensal · Anual · Sempre. Ativo = fundo `text`,
  texto `bg` (funciona nos dois temas).
- Cartão de estatísticas (sem toque).
- Botão grande "Tocar seu Replay" (ícone play, chevron) → toca faixa aleatória.
- Seção "Músicas mais tocadas": linhas com número (1º item em vermelho `#e5352b`), capa 56 px,
  título, subtítulo e contagem de reproduções.

### 3.6 Pesquisa (`search`)

- Campo de busca em pílula (48 px, raio 14, ícone lupa, placeholder "Artistas, músicas ou letras...").
- Chips: "YouTube" (ativo) e "Biblioteca". Ativo: fundo `accent`, texto `on-accent`.
- Seção "Pesquisas recentes" (+ "Limpar").
- Lista de cartões: capa 40 px, título, "artista • álbum", chevron. Toque = toca a faixa.

### 3.7 Configurações (`settings`)

- Botão voltar, título "Configurações", campo de busca (filtra as linhas pelo texto; grupos e
  rótulos sem linhas visíveis somem junto).
- Grupos (ordem exata):
  1. **(sem rótulo)**: Conta e integrações (subtítulo `@cuscusdocerael-o1h`) → `conta`;
     Ouvir junto → `ouvir`.
  2. **QUALIDADE DE ÁUDIO**: Fontes → `fontes`; Transmissão direta da fonte (toggle `directStream`,
     abre `transmissao`); No Wi-Fi (tag "EM USO", valor = `320k/Hi-Res/160k/Auto`) → `qualidade`;
     Nos dados móveis (valor = `Alta/Padrão/Automático`) → `dados`; Dolby Atmos (desabilitado,
     opacidade 50%) → `dolby`.
  3. **GERAL**: Aparência → `appearance`; Downloads (valor "2.4 GB") → `downloads`;
     Reprodução (valor = transição `0s…8s`) → `reproducao`; Normalização de volume (toggle
     `normalize`) → `normalizacao`; Content Preferences → `content`; Desempenho → `desempenho`;
     Música local → `musica-local`; Armazenamento → `armazenamento`; Backup e restauração →
     `backup`; Diversos → `diversos`; Idioma → `idioma`.
  4. **SOBRE E PRIVACIDADE**: Histórico de reprodução e buscas (toggle `history`) → `historico`;
     Limpar histórico recente → `limpar-historico`; Atalhos de teclado → `atalhos`;
     Versão do kazzu → `versao`.
  5. **(sem rótulo)**: kazzu para Wear OS ("Nenhum relógio") → `wear`.
- Rodapé: "kazzu · v1.4.2 / Código aberto · Licença MIT" em `text-2`.
- **Regra de toque:** tocar na linha abre a sub-tela. Tocar **no toggle** só alterna o toggle.

### 3.8 Sub-telas de ajustes (`settings-page`)

Uma tela genérica montada a partir de `SETTINGS_PAGES[key]`. Tipos de item:
- `info`: título + subtítulo, não clicável.
- `action`: título + subtítulo + chevron. Executa `fn` (seção 6) ou abre outra sub-tela (`page`).
- `toggle`: título + subtítulo + interruptor ligado a `STATE.toggles[key]`.
- `radio`: uma linha por opção; a escolhida mostra ✓ (cor `accent`). Grava em `STATE.radio[group]`.

**Desempenho** (`desempenho`): radio `perf` (Economia de bateria / Equilibrado / Alto desempenho)
e uma linha de informação "O que muda" explicando cada modo (seção 7.6).

### 3.9 Aparência (`appearance`)

- Botão voltar, título "Aparência", subtítulo "Material You: as cores seguem a capa da música.".
- Cartão "ESQUEMA ATUAL": 4 amostras (primary, tertiary, primaryContainer, surfaceContainerHigh) e,
  abaixo, uma **prévia de vidro** (uma barra de vidro de 52 px sobre o cartão) que reflete os
  controles da seção 5.3 ao vivo.
- Grupo com 4 toggles: **Cor da capa (You)** (`dynamicColor`), **Tema escuro** (`darkTheme`),
  **Formas expressivas** (`expressiveShapes`), **Animações expressivas** (`animations`).
- **Rótulo "Vidro e textura"** + grupo com 5 controles deslizantes (seção 5.3).

### 3.10 Player expandido (`player`, tela cheia)

Sobe de baixo (`translateY`, 0,38 s; sem transição se "Animações expressivas" estiver desligado).
Fechado = invisível. Fundo = `tint` da faixa atual no tema escuro; no tema claro, `bg-player`.

**Não rola.** O player é um layout fixo que se ajusta à altura disponível. Os blocos se
distribuem com `space-between`. A capa é a única parte que muda de tamanho (seção 7.2).

De cima para baixo:
1. **Barra** (44 px; 40 px em telas baixas): botão recolher (chevron para baixo, 40 px) ·
   segmentado **Áudio | Vídeo** com ícone (fone / câmera), ativo = fundo `text`, texto `bg`.
   Vídeo só mostra o aviso "indisponível"; Áudio continua ativo · espaço de 40 px que equilibra o recolher.
2. **Palco** quadrado 1:1 com lado `--cover` (seção 7.2), centralizado, raio = raio de cartão.
   O visual tem **três modos** (`vinyl_mode`, seção 7.8): `disc` (padrão), `both` e `cover`:
   - **Capa** com rótulo do álbum em pílula escura (`rgba(0,0,0,0.6)`, texto branco 13 px bold) a
     12 px da borda de baixo. **Toque na capa = alterna a letra.** Se a faixa tiver imagem real
     (`image`), ela substitui a capa gerada. Em `both`, a capa ocupa 66% do palco; em `cover`, 100%;
     em `disc`, fica invisível.
   - **Disco de vinil** com a capa da faixa como rótulo e, em `disc`, **agulha** (seção 7.8). Em `disc`,
     **toque no disco = alterna a letra**.
   - **Cartão de letra** no mesmo lugar (seção 7.4): cabeçalho "LETRA" (12 px, `accent`) + botão
     "Voltar à capa ✕". Sem rolagem. Sem letra = "Letra não cadastrada para esta música". Capa e
     letra trocam com opacidade + escala 0,95 → 1 (0,3 s).
3. **Título e ações** em linha: título (27 px, extra-negrito, 1 linha) e artista (14 px, `text-2`);
   à direita **compartilhar** (círculo 40 px) e **curtir** (40 px, raio 12; ativo = fundo `text`,
   ícone `bg` preenchido).
4. **Ilha de controles** (compacta, v1.6: raio de cartão, vidro, borda 1 px, padding 10/12/10, espaço 6 px;
   altura total ≈ 205 dp, antes ≈ 278 dp; ≈ 184 dp com altura < 760 dp e ≈ 169 dp com altura < 660 dp):
   1. **Waveform de progresso** (40 px; 34 px abaixo de 760 dp de altura; 30 px abaixo de 660 dp; variável `--sh`): ~72 barras. Tocadas em
      `accent`, restantes em `text` a 28%. O `input range` fica por cima, transparente, com marcador de
      3 px de largura e altura `--sh − 14 px`, **sempre centralizado na altura da waveform** (margem
      superior fixa de 7 px). Tempos (12 px): atual à esquerda, restante com "-" à direita.
   2. Controles (linha de 54 px; 48 e 44 px em telas baixas): aleatório (34 px) · anterior (40 px) · **play/pause (50 px; 46 e 42 px em telas baixas, fundo `text`,
      ícone `bg`)** · próxima · repetir. Modos ligados = `accent-2`. O play deforma o raio para 30%
      ao ser pressionado (modo expressivo).
   3. Volume: ícone baixo · slider (trilho 4 px, preenchimento `text`, bolinha 10 px, valor 0–100) ·
      ícone alto.
   4. Ferramentas (**5 botões** em 5 colunas, altura 36 px; 32 e 30 px em telas baixas, raio de tile): Fila (abre gaveta) ·
      Letra (fundo `accent-container` enquanto a letra está aberta) · Temporizador (aviso) · **Mais** (abre o menu da seção 3.13) · **Saída de áudio** (ícone de aparelho; aviso "Saída: alto-falantes do aparelho").
   A pílula "Este aparelho" deixou de existir como linha própria (era a maior economia de altura).

**Capa imersiva:** atrás da capa há um **glow** (a própria capa desfocada 34 px, saturada, opacidade
0,65; em faixa tocando, respira só a opacidade entre 0,65 e 0,9 em 4,5 s). A capa reage ao ponteiro
com tilt 3D de até 9° e reflexo iridescente (somente com mouse). Em Slint, ver §11.

**Sliders:** o preenchimento é um elemento próprio (largura = `th/2 + (100% − th) × valor`), não um
degradê, para alinhar com a bolinha.

### 3.11 Fila (`queue`)

- Gaveta que sobe de baixo, altura **60%**, raio 24 (token `radius-card`) em cima, borda superior 1 px,
  fundo `card`. **Alça de arrasto** (36 × 4 px) no topo, como folha inferior M3. Fecha-se deslizando
  105% para baixo.
- Cabeçalho "Fila de reprodução" (18 px, extra-negrito) + botão fechar (ícone ✕, 32 px).
- Lista de **linhas em cartão** (espaço 6 px, raio de tile, fundo `text` a 12%): capa 40 px · título ·
  artista · duração (tabular). Faixa atual: fundo `accent-container`, borda `accent`, **título em
  `text`** (não `accent-2`, por contraste), com prefixo "▶" em `accent`. Toque = toca e fecha a fila.

### 3.12 Toast

Faixa escura flutuante (`rgba(20,14,22,0.92)`, texto **branco fixo**), 2,2 s, a `112 px + área segura`
da base. Não bloqueia toque (`pointer-events: none`).

### 3.13 Menu "Mais" do player (`more`)

Gaveta que sobe de baixo (mesmo estilo da fila, altura pelo conteúdo, `translateY` 0,35 s) com fundo escurecido
atrás (toque fora fecha). Conteúdo:
1. Título "Mais opções" (18 px, extra-negrito).
2. Legenda "Visual do player" e **segmentado de 3 opções** (fundo `p-glass`, raio pílula; ativa = fundo `text`,
   texto `bg`): **Só disco** · **Capa + disco** · **Só capa**. Toque aplica na hora, sem fechar o menu.
3. Linhas "Adicionar à playlist", "Ver álbum" e "Equalizador": fecham o menu e mostram aviso "em breve".

---

## 4. Modelo de dados

```
Track      { id, title, artist, album, duration (s), pos (s, inicial), art (cor da capa),
             tint (cor do player), motif (0–4), lyrics: [string],
             image? (caminho de capa real; se existir, substitui a capa gerada),
             times? ([s], um por linha de letra; se existir, habilita a sincronia real) }
Palette    { key, name, tag, accent, accent2 }
Category   { title, sub, color, track (id) }
StatCard   { eyebrow, big, caption, who, meta, is_text: bool }
Playlist   { title, sub, art, glyph }      Artist { title, sub, art, glyph }
TopSong    { title, sub, plays, art, motif }
SettingsPage { title, items: [Item] }
Item       = Info{title, sub} | Action{title, sub, fn | page} | Toggle{key, title, sub} | Radio{group, title?, options: [string]}
GlassCtrl  { key, title, sub, min, max, unit }   (Aparência > Vidro e textura)
```

**Capas:** geradas (SVG com cor sólida + um dos 5 motivos: círculos, listras, triângulo, grade, nota).
O nome do álbum aparece num rótulo sobre a capa no player. Em Slint, substitua por `Image` de arquivo
quando tiver as capas reais (seção 12).

---

## 5. Estado (`STATE`)

```
screen            : tela atual
history           : pilha de telas (para voltar)
settings_page     : chave da sub-tela aberta
toggles           : { dynamicColor, darkTheme, expressiveShapes, animations,
                      directStream, normalize, history,
                      srcStream, srcDownloads, srcLocal, saveData, dlWifi, explicit, autoplay }
radio             : { audio, mobile, dlq, cross, perf, lang }   (índice da opção)
glass             : { strength, blur, transparency, bright, grain }   (seção 5.3)
replay_tab        : 0–3
player            : { index, pos, playing, shuffle, repeat, liked, lyrics_open, volume }
queue_open        : bool
vinyl_mode        : 'disc' | 'both' | 'cover'   (visual do player; inicial 'disc'; não persiste)
artist_page       : conjunto atual de "Artistas" no Início (≥ 0)
scheme_seed       : cor usada pelo esquema atual (capa da faixa)
suggest_page      : conjunto atual de "Músicas sugeridas" (≥ 0)
lyric_idx         : índice da linha de letra ativa (derivado de pos; não persiste)
```

**Valores iniciais:** `audio=0`, `mobile=0`, `dlq=0`, `cross=2` (4 s), `perf=1` (ou **0** se o aparelho
for modesto, ver 7.6), `lang=0`, `replay_tab=3`, `index=0` (Creep), `pos=91`, `volume=70`, todos os
toggles de aparência ligados, `glass = { strength: 45, blur: 24, transparency: 48, bright: 107, grain: 36 }`.

### 5.3 Vidro e textura (painel de Aparência)

| Controle | Faixa | Padrão | Efeito |
|---|---|---|---|
| Intensidade do vidro | 0–100% | 45 | força do realce de borda e do reflexo (`--gl-rim` = valor / 100) |
| Desfoque | 0–40 px | 24 | desfoque do fundo atrás do vidro (`--gl-blur`) |
| Transparência | 0–100% | 48 | opacidade da base do vidro (`--gl-tint` = 92% − valor × 0,8) |
| Brilho | 80–130% | 107 | claridade do vidro (`--gl-bright` = valor / 100) |
| Granulação | 0–100% | 36 | opacidade do grão do app (`--gl-grain` = valor / 100 × 0,25) |

Os controles são deslizantes nativos com `accent-color`. Arrastar **não repinta a tela**: só
atualiza as variáveis e o texto do valor. Os valores não são persistidos (o app ainda não tem
armazenamento). Em modo "Economia de bateria", o vidro fica sólido mesmo com desfoque > 0.

---

## 6. Ações (nomes que a UI chama)

| Ação | Efeito |
|---|---|
| `navigate(screen)` | empilha a tela atual e abre a nova |
| `back()` | desempilha e volta (padrão `home`) |
| `playTrack(id)` | define a faixa, zera `pos`, toca |
| `playRandom()` | toca uma faixa aleatória |
| `togglePlay()` | alterna play/pause (liga/desliga o tick de 1 s) |
| `next()` | próxima faixa (aleatória se `shuffle`; zera `pos`) |
| `prev()` | se `pos > 4 s`: zera a faixa atual; senão, anterior |
| `seek(percent)` | `pos = percent/100 × duration` |
| `setVolume(v)` | grava `volume` |
| `toggleMode(shuffle\|repeat)` | liga/desliga o modo |
| `toggleLike()` | curtir / descurtir |
| `toggleLyrics()` | mostra/oculta a letra sobre a capa |
| `renderLyrics()` | monta as linhas da letra da faixa atual (uma vez por faixa) |
| `updateLyrics()` | calcula a linha ativa e aplica foco e desfoque; centraliza a linha |
| `openPlayer()` / `closePlayer()` | abre/fecha o player expandido |
| `openQueue()` / `closeQueue()` | abre/fecha a fila |
| `openMore()` / `closeMore()` | abre/fecha o menu "Mais" do player (monta com `renderMore()`) |
| `setVinylMode(m)` | grava `vinyl_mode` (`disc`, `both`, `cover`), reaplica as classes e repinta o menu |
| `applyVinylMode()` | liga `vinyl-only` (disco) ou `cover-only` (capa) no player; `both` = nenhuma |
| `setArtistPage(n)` | troca o conjunto de artistas do Início (≥ 0), com animação de deslizar |
| `updateProgress()` | além de barra e tempos, grava `--prog` (0–1) no player, que posiciona a agulha |
| `openSettingsPage(key)` | abre a sub-tela `key` |
| `toggleSetting(key)` | inverte `toggles[key]` e repinta a tela atual |
| `setRadio(group, index)` | grava `radio[group]`; se `perf`, reaplica os modos de desempenho |
| `setGlass(key, value)` | grava `glass[key]`, atualiza o texto do valor e aplica as variáveis do vidro |
| `applyGlass()` | escreve `--gl-tint`, `--gl-blur`, `--gl-bright`, `--gl-rim`, `--gl-grain` |
| `applyScheme(seed)` | recalcula o esquema Material 3 a partir da semente e aplica em todo o app |
| `syncVisualToggles()` | aplica cor, tema, formas, movimento, vidro e modo de desempenho |
| `setReplayTab(i)` | troca o período do Replay |
| `filterSettings(text)` | mostra só as linhas que contêm o texto; esconde grupos sem linhas |
| `comingSoon()` | toast "Em breve nesta versão" |
| `scanLibrary()` | toast de biblioteca atualizada (simulado) |
| `clearCache()` | toast "Cache limpo" (simulado) |
| `clearSearchHistory()` | toast "Histórico apagado" (simulado) |
| `share()` | toast "Ouvindo … no kazzu" (em Slint, usar o compartilhamento do sistema) |
| `showToast(msg)` | toast por 2,2 s na parte de baixo |

---

## 7. Comportamentos importantes

### 7.1 Tick e reprodução
- **Tick do player:** a cada 1 s, `pos += 1` enquanto tocando. Ao chegar em `duration`: se `repeat`,
  volta para 0; senão, chama `next()`.
- **Mini player e player expandido** leem o mesmo estado; `renderPlayerUI` é a única função que pinta o player.
- **Barra do mini player** = `pos / duration` em largura.
- **Cor de fundo do player** = `tint` da faixa atual no tema escuro; no tema claro, `bg-player`.
- **Letra aberta** é estado (`lyrics_open`): continua aberta ao trocar de faixa e ao fechar/reabrir o player.
- **Volume** reflete `volume` sempre que o player é repintado.
- **Troca de faixa** recalcula o esquema (se "Cor da capa" estiver ligada). Dock, avatar, chips,
  toggles, bolha e fundo do player seguem o esquema.

### 7.2 Player sem rolagem e altura da capa (medido)

A capa é o único bloco que muda de tamanho. Fórmula em CSS (o mesmo cálculo vale no Slint com a
altura disponível em dp):

| Faixa de altura da área do app | Lado da capa (`--cover`) | Altura fixa dos outros blocos |
|---|---|---|
| > 760 dp | `clamp(180, min(86% da largura, altura − 366), 380)` | ~366 dp |
| ≤ 760 dp | `clamp(180, min(86% da largura, altura − 330), 380)` | ~330 dp |
| ≤ 660 dp | `clamp(150, min(86% da largura, altura − 308), 380)` | ~308 dp |

Medidas dos aparelhos de referência (capa e overflow; **overflow 0 em todas**):

| Tela (dp) | Capa (dp) | Overflow |
|---|---|---|
| 320 × 568 | 260 | 0 |
| 360 × 640 | 310 | 0 |
| 360 × 740 | 310 | 0 |
| 360 × 800 | 310 | 0 |
| 390 × 700 | 335 | 0 |
| 390 × 780 | 335 | 0 |
| 412 × 915 | 354 | 0 |
| 430 × 932 | 361 | 0 |
| 600 × 960 (janela desktop 92vh) | 361 | 0 |

Com a ilha compacta (v1.6) a capa passou a ser limitada pela **largura** na maioria das telas; a altura que sobra
vira espaço entre os blocos (`space-between`). Em 320×568 a capa cresceu de 201 para 260 dp.

Antes desta revisão a capa era 290 fixos, e em 360×640 o player rolava (717 px de conteúdo em 640).

**Regras que sustentam o encaixe:**
- A ilha (waveform, controles, volume, ferramentas com saída de áudio) é fixa em altura. Nas telas baixas, reduz
  paddings e alturas, sem remover controles.
- O player não tem `overflow` visível. Nenhum bloco some: se algo não coubesse, seria cortado, por isso
  o teste de overflow é obrigatório (seção 13).

### 7.3 Modo de tela e áreas seguras
- A área do app ocupa a tela inteira no Android (altura `dvh`). A barra de status e a barra de gestos
  são áreas seguras (`--safe-t`, `--safe-b`, vindas dos insets do sistema):
  - Telas: padding superior `16 px + safe-t`, inferior `170 px + safe-b`.
  - Ilha: `10 px + safe-b` da base.
  - Player: padding superior `max(14 px, safe-t)` e inferior `max(20 px, safe-b)`.
  - Toast: `112 px + safe-b`.
- Corte de câmera, dobradiça e cantos arredondados: os insets de `displayCutout` entram no mesmo cálculo.
- Tablet ou janela ≥ 440 px: o app vira um quadro centralizado de até 420 × 900 com cantos de 40 px.

### 7.4 Letra imersiva
- **Estrutura:** a letra é uma coluna de linhas dentro do cartão da capa. A linha ativa fica no meio
  do cartão, em peso 800 e largura 112 (wdth). As linhas à frente e atrás ficam em opacidade 26% e
  desfocadas (até 3 px, crescendo com a distância). Bordas do cartão desvanecem (máscara de 16%).
- **Sincronia:**
  - Se a faixa tiver `times` (segundos, um por linha): a linha ativa é a última cujo tempo é ≤ `pos`.
    Tocar numa linha pula para o tempo dela.
  - Sem `times`: as linhas são distribuídas pela duração (`idx = floor(pos / duration × n)`). É uma
    **aproximação provisória**. A letra não acompanha a música de verdade até haver tempos reais.
- **Sem rolagem.** O usuário não rola a letra. A troca de linha anima a posição do bloco inteiro.
- **Sem letra:** "Letra não cadastrada para esta música" centralizado.
- **Conteúdo:** `lyrics` está vazio em todas as faixas de exemplo. Cadastre apenas letras licenciadas
  (ou links). Não copie letras de músicas protegidas.

### 7.5 Capas com vinil, toca-discos, tilt e reflexo
- **Capas com vinil:** item de 140 × 100 dp (capa 100 dp, disco 92 dp a 48 dp da esquerda), margem de
  8 dp, `scroll-snap` central, sem sobreposição entre itens (o passo é 156 dp). Para cada item,
  `d = (centro do item − centro da lista) / 156`, `scale = 1 − 0,18·min(|d|,1)`,
  `opacity = 1 − 0,5·min(|d|,1)`. Disco: preto com sulcos (anéis de 1,5 dp) + brilho cônico fraco; rótulo
  central redondo (38%) com a capa e furo de 6 dp.
- **Camadas:** o carrossel e o toca-discos formam um grupo isolado (nenhum item passa de `z-index` 0 fora do
  grupo), então nunca cobrem a ilha inferior, o player aberto nem a fila. O voo do disco acontece dentro do grupo.
- **Toca-discos (3D por camadas):** base 230 × 142 dp, raio 18, metal escovado escuro nos dois temas
  (gradiente 160° `#4d4d57 → #232329` + linhas verticais de 1 dp a 4%). **Profundidade:** realce interno de
  1,5 dp no topo, lateral sólida de 8 dp (`0 8px 0 #18181d`) e sombra projetada de 20 dp. A profundidade é feita
  só com camadas e sombras; **o prato não é inclinado em 3D**, porque o voo do disco usa as medidas reais do
  prato e uma inclinação o faria pousar fora do lugar.
  - **Prato:** 100 dp à esquerda; borda cromada com marcas de estroboscópio (cônico de 2° em 2°), tapete de
    borracha (anéis de 2 dp) e pino central cromado de 8 dp por cima do disco. Disco de 92 dp encaixado, com
    sombra de elevação e **reflexo cônico parado** por cima (o disco gira, o reflexo não).
  - **Braço:** 98 dp com pivô cromado de 22 dp à direita, tubo metálico de 4 dp, **contrapeso** (12 × 18 dp)
    atrás do pivô e **cabeça** (10 × 16 dp) na ponta, sombras projetadas. Em repouso: 22°, apoiado no **suporte**
    (10 × 14 dp); tocando: 72° (transição de 0,7 s).
  - **Controles (decorativos, exceto o botão):** LED (apagado `#5a1d1d`; tocando `#4cff8a` com brilho),
    **botão** de 18 dp (toque = mesmo efeito de tocar o prato; gira 60° tocando), controle de pitch de 48 dp e
    rótulo "33⅓".
  - **Rotação:** 1,8 s por volta (33⅓ rpm), só enquanto toca a faixa que está no prato.
- **Sequência ao abrir (≈1,2 s):** 1) o disco desliza 46 dp para fora da capa (0,46 s);
  2) voa em arco até o prato, girando 360° (0,72 s);
  3) a faixa começa e o braço desce. Devolver: o braço sobe (0,45 s) e o disco volta em arco
  (0,64 s) e entra na capa. Com "Animações expressivas" desligado ou Economia de bateria, tudo
  acontece sem movimento.
- **Tilt da capa:** só com mouse. Até 9° em X e Y, e reflexo iridescente deslizando (`--sx`).
  Desligado em Economia de bateria.

### 7.6 Modos de desempenho

Ajuste em **Configurações › Desempenho › Modo**.

| Modo | Vidro | Fundo animado | Grão | Glow | Reflexos e tilt | Morph e magnetismo | Telas |
|---|---|---|---|---|---|---|---|
| **Economia de bateria** (`perf = 0`) | sólido (94%), sem desfoque | parado e oculto | oculto | oculto | oculto | oculto | sem animação de entrada |
| **Equilibrado** (`perf = 1`, padrão) | desfoque pela metade | parado | conforme o slider | parado (sem respiração) | só tilt | ativos | ativas |
| **Alto desempenho** (`perf = 2`) | completo | animado | conforme o slider | respirando | ativos | ativos | ativas |

**Início automático:** se o aparelho tiver **≤ 4 núcleos** ou **≤ 3 GB de RAM** (estimativa do
sistema), o app começa em **Economia de bateria**. O usuário pode trocar depois, e a escolha vale
até fechar o app (não é persistida).

**Por que é reduzido de verdade:** antes desta revisão, o seletor não mudava nada. Agora cada modo
troca as classes do app (`perf-low`, `perf-mid`) e as animações caras são desligadas no nível de CSS.

### 7.7 Animações: o que muda e o que fica
- **Mantido (com motivo):** entrada de tela (0,55 s) e de palavras do hero (uma vez por tela); pílula
  do dock (0,6 s, mola); morph da capa entre mini player e player; elástico do slider ao arrastar;
  toque com escala (0,92–0,955) e mola no retorno.
- **Removido:** respiração da capa com transformação (era repintada a cada quadro sobre um elemento
  de 34 px de desfoque). Agora só a opacidade anima, e só enquanto toca, e só em Alto desempenho.
- **Limitado:** magnetismo (desloca botões até 22% em direção ao ponteiro) só em play, anterior,
  próxima e bolha de pesquisa. Dock, ícones de barra e voltar não se movem mais.
- **Desligado** com o toggle "Animações expressivas" ou com `prefers-reduced-motion`.
- **Exceção (essencial ao visual):** o **giro do disco** (player e toca-discos) e a **agulha** continuam animando
  com o toggle desligado e com `prefers-reduced-motion`. O giro só para quando a música está pausada.

### 7.8 Player: disco de vinil e agulha

Três visuais (`vinyl_mode`), escolhidos no menu Mais (seção 3.13). Tamanhos em % do palco (lado `--cover`):

| Modo | Capa | Disco | Agulha |
|---|---|---|---|
| `disc` (padrão) | invisível (mantida para o morph) | 86%, esquerda 0, topo 7%, rótulo com 22% de margem | visível |
| `both` | 66%, esquerda 0, topo 17% | 66%, esquerda 34%, topo 17%; recuado 6% quando pausado e totalmente saído tocando (0,7 s, mola) | oculta |
| `cover` | 100% | oculto | oculta |

- **Disco:** preto com sulcos (anéis de 1,5 dp) e borda interna escura; rótulo redondo com a capa da faixa; furo
  de 10 dp. **Giro:** 3,6 s por volta, linear, infinito; pausa **congela no ângulo atual** (não volta a zero).
  O disco tem marcas que giram junto (reflexo cônico e uma bolinha clara no rótulo), pois capas simétricas
  não deixavam o giro visível. Há também um reflexo **parado** por cima. Sombra projetada de 22 dp (removida em Economia).
- **Agulha** (só em `disc`): pivô cromado de 30 dp em (90%, 6%) do palco; tubo de 6 dp com comprimento
  `0,52 × --cover`; contrapeso e cabeça com agulha. Dois ângulos independentes:
  1. **Descer/subir:** repouso −6° (fora do disco); tocando 16°. Easing `cubic-bezier(.45,.05,.25,1)`, 1,4 s.
  2. **Avanço:** `0° → 22°` proporcional a `pos / duration` (`--prog`), transição **linear** de 1,1 s a cada tick,
     o que dá movimento contínuo sem saltos. Trocar de faixa ou mover o seletor leva a agulha de volta.
  Resultado: borda do disco (16°) até perto do centro (38°). Em Slint, usar duas `rotation-angle` encadeadas
  no mesmo ponto de origem (pivô).
- **Letra aberta:** disco e agulha somem (opacidade, 0,3 s).
- **Toque:** em `disc`, o disco recebe o toque (alterna a letra); nos outros modos quem recebe é a capa.

---

## 8. Consistência de contraste e tema claro

Regras para qualquer tela nova:
1. Texto usa só `text`, `text-2` ou `accent`/`on-accent` (com as regras da seção 2.2).
2. Nunca branco ou preto fixos em fundos que mudam de tema. Exceções aprovadas: rótulo sobre capa
   (`rgba(0,0,0,0.6)` + branco), toast (fundo escuro + branco) e tile de categoria (cor da categoria + branco).
3. Vidro sempre com `text` como cor de conteúdo.
4. Toggle desligado usa `text` a 24%, não branco a 18% (que sumia no tema claro).

### 8.1 Tema (`Theme` global no Slint)

```slint
export global Theme {
  in-out property <color> bg;            // surface
  in-out property <color> bg-player;     // surfaceContainerLow
  in-out property <color> card;          // surfaceContainer
  in-out property <color> card-hover;    // surfaceContainerHigh
  in-out property <color> accent;        // primary
  in-out property <color> on-accent;     // onPrimary
  in-out property <color> accent-2;      // tertiary
  in-out property <color> accent-container;
  in-out property <color> text;          // onSurface
  in-out property <color> text-2;        // onSurfaceVariant
  in-out property <color> text-3;        // outline (só ícones)
  in-out property <color> line;          // outline (divisórias)
  // vidro
  in-out property <float> gl-tint;       // 0..1
  in-out property <length> gl-blur;      // dp (ver 11)
  in-out property <float> gl-bright;
  in-out property <float> gl-rim;
  in-out property <float> gl-grain;
  // áreas seguras e medidas
  in-out property <length> safe-t;
  in-out property <length> safe-b;
  in-out property <length> cover;        // lado da capa do player (7.2)
  in-out property <bool> expressive;
  in-out property <int> perf;            // 0 economia, 1 equilibrado, 2 alto
}
```

O esquema é calculado fora do Slint (Rust + `material-color-utilities`) a partir da semente e copiado
para `Theme` a cada troca de faixa.

---

## 9. Itens de exemplo / pendências (não são funcionalidades reais)

- **Letras:** `lyrics` está vazio em todas as faixas. Cadastre apenas letras licenciadas (ou links).
  A sincronia real precisa de `times` por linha (ver 7.4).
- **Replay:** os números das abas (Semanal, Mensal etc.) são de exemplo; só "Sempre" tem dados reais.
  Ligue `replay_tab` a uma consulta de histórico quando existir.
- **Ações simuladas:** Conta/Last.fm/Discord, Ouvir junto, Backup, Wear OS, Escanear biblioteca, limpar
  cache e histórico mostram aviso (`comingSoon`). Implemente de verdade quando houver backend.
- **Vídeo:** indisponível (aviso).
- **Compartilhar:** no HTML mostra toast; no app, usar o compartilhamento do sistema.
- **Capas:** geradas por código como placeholder. Capas reais entram pelo campo `image` (seção 12).
- **Vidro e textura:** os valores não são salvos entre sessões (o app ainda não tem armazenamento).

---

## 10. Camada visual FX (kazzu0_4.html)

Toda a camada está em dois blocos marcados com `FX` (final do `<style>` e do `<script>`), além das
funções de base que ela envolve. Apagar os dois blocos devolve o visual base (sem vidro extra,
sem mesh, sem letra imersiva). Respeita o toggle "Animações" e `prefers-reduced-motion`.

### 10.1 Fundo ambiente
- **Mesh gradient:** 3 manchas radiais desfocadas (28 px) com `--m1` = cor da capa (`art`, ou `accent`
  se "Cor da capa" estiver off), `--m2` = `accent-2`, `--m3` = `accent-container`. As cores animam
  1,4 s ao trocar de faixa; o conjunto deriva devagar (38 s) e sobe 8% da rolagem (parallax).
  Opacidade 0,7 no escuro e 0,45 no claro. **Modo Equilibrado e Economia:** parado.
- **Iluminação cinematográfica:** luz suave no topo e vinheta embaixo.
- **Grão:** ruído fractal em `overlay`, opacidade = `--gl-grain` (padrão 9%). Desligado em Economia.
- No desktop (≥ 440 px) manchas de cor também aparecem fora do "aparelho".

### 10.2 Liquid Glass (mini player, dock, bolha, pílula do topo, ilha de controles, segmentado, prévia)

- **Base:** brilho vertical branco 17% → 4% sobre `card-hover` a `--gl-tint`; desfoque `--gl-blur`,
  saturação 185%, brilho `--gl-bright`. Em telas de toque, desfoque × 0,75.
- **Realces internos** (intensidade = `--gl-rim`):
  - linha clara no topo (`rim + 12%`), linha fraca na base (`rim × 18%`);
  - **brilho interno nas bordas** (sombra interna de 12 px, `rim × 22%`): é a aproximação da refração
    de borda. A luz "se acumula" perto da borda, como em um vidro grosso;
  - **aberração cromática** sutil nas laterais (1,5 px rosa à esquerda, ciano à direita, 15%).
- **Brilho especular** que segue o ponteiro/dedo (`--mx`, `--my`) e **borda iridescente**
  (cônico rosa → azul → verde, 1 px, 50%) que gira com a posição horizontal (`--ang`).
- **Glass stacking:** a ilha de controles tem uma 2ª camada de vidro na linha de botões
  (`rgba(255,255,255,0.05)`, raio 28); o dock tem a pílula ativa como 3ª camada.
- **Limite:** isto **não** é refração real. Refração real distorce o fundo (ver §11).

### 10.3 Ilhas flutuantes e morph
- Mini player e dock flutuam a 10 px das bordas com sombra dupla (`0 14px 34px` e `0 2px 6px`).
- **Pílula do dock:** um único elemento (altura 46, raio 18) que anima `translateX` e `width` até o
  botão ativo (0,6 s, mola).
- **Elemento compartilhado:** ao abrir/fechar o player, a capa do mini player vira a capa grande (e
  vice-versa) via View Transitions (`kz-cover`, 0,6 s). Sem suporte, vale a subida clássica (0,38 s).
  Em Slint: animar `x/y/width/height` de uma única `Image` entre as duas posições.

### 10.4 Blur progressivo
- Duas faixas fixas: **topo** (70 px + área segura, aparece só depois de 6 px de rolagem) e **base**
  (168 px, sempre).
- 3 camadas com máscaras escalonadas: 2 px (até 100%), 6 px (até 70%) e 16 px (até 42%); 10 px na
  camada forte em telas de toque. Em Economia: faixa de base com fundo em gradiente (sem desfoque).

### 10.5 Capas com vinil, bento e tipografia
- Carrossel e bento: ver 3.2, 3.3 e 7.5.
- **Squircle:** `corner-shape: squircle` onde suportado (ver 2.4).
- **Tipografia editorial:** hero e títulos de seção (ver 2.3). A entrada do hero revela palavra por
  palavra (0,9 s, atraso 70 ms por palavra).

### 10.6 Player: waveform, glow, tilt e microinterações
- **Waveform:** 72 barras determinísticas por `id` da faixa (hash → ruído suavizado, 16%–100% da altura),
  espaçamento 3 px, canvas em DPR ≤ 2, redesenhado a cada tick e ao arrastar. Substituir por picos reais
  do áudio quando existirem.
- **Glow:** capa desfocada 34 px, saturação 1,9, opacidade 0,65.
- **Tilt/parallax:** ver 7.5.
- **Marcador da waveform:** 3 px de largura, altura `--sh − 14 px`, margem superior 7 px. Com `--sh` = 46 px
  (padrão) ou 38 px (telas baixas) o marcador fica centrado na waveform nos dois casos. Antes, a margem fixa de
  7 px deixava o marcador 4 px abaixo do centro nas telas baixas.
- **Sliders elásticos:** ao arrastar, o slider escala 1,02 × 1,28 (waveform 1,02 × 1,12) com mola;
  ao soltar volta com overshoot.
- **Molas:** `--spring` = `linear(...)` (fallback `cubic-bezier(0.34, 1.56, 0.64, 1)`); pressionar = escala
  0,92–0,975; telas entram com 0,55 s.

### 10.7 Desempenho e compatibilidade
- Animações pesadas (mesh, glow) rodam só quando visíveis e fora dos modos de economia.
- Carrossel e tilt atualizam em `requestAnimationFrame`.
- `-webkit-box-reflect`, `corner-shape` e View Transitions são aprimoramentos progressivos: sem
  suporte, o layout continua correto.
- Requisito mínimo para a versão web de referência: WebView/Chrome com `cqw`/`cqh` (Chrome 105+).

---

## 11. Preparação para Slint: o que é nativo, o que é aproximado, o que exige shader

### 11.1 Classificação dos efeitos

Legenda:
- **Nativo:** Slint faz diretamente com as propriedades padrão.
- **Aproximação nativa:** Slint faz, com uma troca visual documentada.
- **Renderização personalizada:** exige código extra (imagem pré-renderizada no Rust ou shader).
- **Remover no Android:** sem ganho real; não implementar.

| Efeito | Classe | Como fazer em Slint | Alternativa visual (preserva a identidade) |
|---|---|---|---|
| Cores, tokens, raios, bordas 1 px | Nativo | `Rectangle` com `background`, `border-radius`, `border-width`, `border-color` | — |
| Cards, tiles, grupos, pílulas | Nativo | `Rectangle` + `TouchArea` | — |
| Toggles, chips, segmentado | Nativo | componente próprio (não o `Switch` padrão) | — |
| Listas, carrosséis, grade | Nativo | `ScrollView` / `Flickable` (`viewport-width` para carrossel) | — |
| Pílula do dock (morph) | Nativo | `animate x, width` com easing | — |
| Elemento compartilhado (capa) | Nativo | `animate x, y, width, height` em uma `Image` | — |
| Fila (gaveta) e player | Nativo | `animate y`; `visible` no fim da animação | — |
| Toast | Nativo | `opacity` animado + `Timer` de 2,2 s | — |
| Waveform | Nativo | `Path` ou `Rectangle` em `for` (72 barras), cor por `index / 72 ≤ progresso` | — |
| Ícones | Nativo | SVG em `Image` (paths de `ICONS`) ou fonte de ícones | — |
| Sombra suave (cards, mini, dock) | Aproximação nativa | `drop-shadow-*` quando disponível na versão do Slint (verificar); senão, borda com opacidade | sombra pré-renderizada em PNG |
| Vidro: base translúcida + borda + realce | Aproximação nativa | `Rectangle` com `background` semi-opaco, `border`, `@linear-gradient` de topo | — |
| Brilho especular que segue o dedo | Aproximação nativa | `Rectangle` com `@radial-gradient` posicionado pelo `TouchArea` | — |
| Realce interno de borda | Aproximação nativa | `@radial-gradient` ou gradiente de borda, mais forte nas bordas | anel PNG pré-renderizado por tamanho de vidro |
| Mesh ambiente (3 manchas) | Aproximação nativa | 3 `Image` circulares pré-renderizadas por cor (radial), animando `x`/`y` lentamente | — |
| Desfoque progressivo das bordas | Aproximação nativa | faixas com `@linear-gradient` de opacidade sobre o fundo sólido `bg` (sem desfoque real) | — |
| Mola (overshoot) | Aproximação nativa | `animate` com `easing: cubic-bezier(0.34, 1.56, 0.64, 1)` (verificar suporte na versão) | — |
| Letra (desfoque por distância) | Aproximação nativa | `opacity` + `scale` por linha; sem desfoque real | — |
| Grão do app | Aproximação nativa | `Image` de ruído 512×512 com `opacity` = `gl-grain`, sem modo de mesclagem `overlay` | — |
| Glow da capa (desfocado 34 px) | Renderização personalizada | pré-renderizar a capa desfocada no Rust (crate `image`, gaussiano) **uma vez por faixa**, cache em memória | — |
| Blur do fundo atrás do vidro (em tempo real) | Renderização personalizada | não existe backdrop-blur nativo na linha 1.x do Slint (verificar a versão). Ver alternativa | **capa desfocada pré-renderizada** atrás do dock e do mini player; o vidro reflete a capa, não o conteúdo que passa |
| **Refração real** (distorção do fundo) | Shader | exige shader próprio (wgpu ou camada personalizada) | **realce interno de borda** (acima) + especular; é a aproximação recomendada |
| Aberração cromática | Aproximação nativa | duas bordas de 1 px, rosa e ciano, opacidade 15% | — |
| Anel iridescente cônico | Renderização personalizada | conic não existe no Slint: pré-renderizar um anel cônico em PNG e girar com `rotation-angle` | gradiente linear de 3 cores estático no anel |
| Squircle (cantos contínuos) | Renderização personalizada | `Path` com superelipse gerado uma vez | `border-radius` normal (o HTML também cai para raio normal onde `corner-shape` não existe) |
| Tilt 3D da capa (perspectiva) | Remover no Android | Slint não tem perspectiva 3D | mover só o brilho especular conforme o dedo |
| Capas com vinil e toca-discos | Nativo | disco = PNG de sulcos pré-renderizado + `Image` redonda no centro; capa = `Image` por cima; voo do disco = `animate x, y, rotation-angle, width`; braço = `rotation-angle` com origem no pivô | — |
| Reflexo de caixa (`box-reflect`) | Renderização personalizada | `Image` espelhada pré-renderizada com gradiente de transparência | opcional; desligado em Economia |
| Morph do dock/capa (View Transitions) | Nativo | animar propriedades (ver acima) | — |
| Modo de mesclagem (`mix-blend-mode`) | Remover no Android | não existe | opacidade normal (o efeito fica um pouco mais claro; o slider de granulação compensa) |
| Fontes variáveis (eixo `wdth` da letra) | Verificar | ver 11.3 | pesos estáticos |

### 11.2 Efeitos que exigem shader (fase 2)

Três efeitos pedem renderização própria e não devem bloquear a versão 1 do Slint:

1. **Refração real do vidro:** o fundo é deslocado pela forma do vidro (lente). É o efeito mais caro.
   Em Slint, precisa de uma camada personalizada que amostre o fundo renderizado. Alvo: só no
   modo Alto desempenho e só no mini player e na ilha.
2. **Blur de fundo em tempo real:** atrás do vidro, desfocar o conteúdo que passa. Alternativa da
   versão 1: capa desfocada pré-renderizada (11.1).
3. **Mesh e glow em tempo real:** hoje são pré-renderizados; só exigiriam shader se a cor da capa
   mudasse animando pixels (não é o caso).

### 11.3 Fontes e letra
- A letra usa o eixo `wdth` (112 no ativo, 100 nos outros). Se a versão do Slint não aplicar eixos
  de fonte variável, use duas fontes estáticas (Roboto Flex 700 e 800) e mantenha só o peso como
  diferença entre linha ativa e inativa.
- Escala de fonte do sistema (texto grande de acessibilidade): até 130%, o player deve continuar
  sem overflow. Títulos longos quebram em 1 linha com reticências; letras quebram em mais linhas e o
  foco continua centralizado.

### 11.4 Tamanhos e unidades
- Use **dp** para todas as medidas (não px físicos). Fontes usam **sp** (com escala do sistema).
- Medidas de tela vêm de `Window` (largura e altura da área útil, menos insets). O lado da capa é
  calculado conforme 7.2 e gravado em `Theme.cover`.

### 11.5 Desempenho no Slint
- Alvo: 60 fps; mínimo aceitável: 30 fps em Economia.
- Nenhum desfoque em tempo real. Toda imagem pesada (capa desfocada, mesh, anel) é gerada uma vez
  e colocada em cache por faixa.
- No máximo 4 camadas animadas ao mesmo tempo (mesh ×3 + glow). Em Equilibrado, o mesh fica parado.
- Waveform redesenhada só no tick de 1 s e ao arrastar, não a cada quadro.
- Modo de desempenho (`Theme.perf`) desliga camadas, não funcionalidades.

---

## 12. Capas reais

- **Campo:** `Track.image`. Se existir, substitui a capa gerada em todos os lugares: tiles, cards,
  capas com vinil, fila, mini player, player (com o glow e o rótulo do álbum), Replay e Explorar.
- **Caminho:** no Slint, `@image-url("capas/…")` em `images/`. A capa gerada continua como reserva se a
  imagem faltar.
- **Validação de composição (checar com capas reais antes de liberar):**
  1. Rótulo do álbum legível sobre capas claras e escuras (pílula 60% preta + branco).
  2. Glow não estoura nem fica sujo com capas muito saturadas.
  3. Capas com vinil: rótulo do disco nítido e bordas sem serrilhado ao escalar.
  4. Mini player: capa de 42 px reconhecível (contraste de borda).
  5. Explorar: miniatura de 44% com cantos legíveis.
- **Direitos:** capas de álbuns são obras protegidas. Use somente imagens que o projeto tenha licença
  para usar (ou arte própria). Não embutir capas copiadas de lojas ou de serviços de streaming.

**Nota de validação desta revisão:** a verificação visual foi feita com capas **sintéticas** (imagem
de teste gerada por código, sem marca nem arte de álbum). Não foi possível testar com capas reais
neste ambiente, porque a rede disponível não libera servidores de imagem. Os cinco pontos acima
precisam ser checados com as capas finais.

---

## 13. Critérios de aceite e testes

A revisão foi validada assim (Chromium headless, viewports de celular). Repetir na implementação Slint:

| Teste | Critério | Resultado nesta revisão |
|---|---|---|
| Overflow do player em 9 tamanhos (320×568 a 600×960) | 0 px | 0 px em todos |
| Erros de JavaScript em 55 ações (troca de faixa, fila, letra, ajustes, filtro, desempenho, vidro) | 0 | 0 |
| Contraste de texto (pares da seção 2.2) | ≥ 4,5:1 | todos ≥ 4,6:1 (claro) e ≥ 7,5:1 (escuro) |
| Contraste de não texto | ≥ 3:1 | todos ≥ 3,5:1 |
| Letra: linha ativa centralizada e linhas distantes desfocadas | visual | conferido com letras de exemplo |
| Vidro: controles mudam a aparência sem repintar a tela | visual | conferido nos cinco controles |
| Modo Economia: sem desfoque de vidro, malha e grão | visual | conferido |
| Tema claro: botão de pesquisa, toggles, cards de estatística e toast | visual | conferido |
| Player em 320×568, 360×640 e 412×915 com seletor de minutagem na ilha | overflow 0 px; marcador centrado na waveform | conferido |
| Disco do player gira tocando e congela pausado, também com animações desligadas e `prefers-reduced-motion` | visual + ângulo muda entre quadros | conferido |
| Três visuais do player pelo menu Mais | `disc`, `both`, `cover` aplicam as classes certas | conferido |
| Início: playlists em cima, 3×3 de músicas, artistas no fim com `>` e `<` | visual | conferido no HTML |

**Como testar em Slint:** repetir os tamanhos da tabela 7.2 no emulador Android (perfis de 320, 360,
390, 412 e 430 dp de largura, com alturas de 568 a 932 dp). Para cada perfil, conferir: o player não
rola; a capa tem o lado da tabela; nenhum botão sai da área segura; a letra centraliza.

**Limites deste teste:** foi feito em Chromium, não no WebView do Android. Não foram medidos
quadros por segundo em aparelho real. Medir no aparelho antes de fechar o desempenho.

---

## 14. Mapeamento sugerido para Slint (resumo)

| No HTML | Em Slint |
|---|---|
| `:root` tokens e `applyScheme` | `global Theme` (seção 8.1) preenchido por Rust |
| `#app` (aparelho) | `Window` com área útil (insets menos barras) |
| Tela (`.screen`) | componente `Screen` com `ScrollView` vertical; a navegação é um `property <string> current` |
| Pilha de telas | `property <[string]> history` + funções `navigate` / `back` na raiz |
| Ilha inferior | `VerticalLayout` com mini player, dock e bolha |
| Mini player | `Rectangle` com vidro (11.1), capa 42 px, `TouchArea` que abre o player |
| Dock | `HorizontalLayout` com `TouchArea` por botão; pílula com `animate x, width` |
| Bolha de pesquisa | `Rectangle` circular 58 px com `TouchArea` |
| Grade de tiles | `GridLayout` 3 colunas; tile = `Image` + `Rectangle` de rótulo |
| Carrossel | `ScrollView` horizontal (`viewport-width`) |
| Capas com vinil | capa + disco, `scale` e `opacity` pela distância ao centro (11.1) |
| Toggle | componente próprio (`Rectangle` + `TouchArea`) |
| Slider de vidro e de volume | `Slider` do Slint, estilizado; preenchimento = `Rectangle` à parte |
| Waveform | `Path` ou `Rectangle`s num `for` (72 barras), alturas vindas de `[float]` por faixa |
| Player | `Rectangle` com `y` animado; layout de colunas com a capa de lado `Theme.cover` |
| Capa e letra | dois elementos sobrepostos; troca animando `opacity` e `scale` |
| Letra | coluna de `Text` com `y` animado para centralizar a linha ativa; sem `Flickable` |
| Fila | `Rectangle` com `y` animado; alça de 36 × 4 dp |
| Toast | `Rectangle` com `opacity` animado + `Timer` de 2,2 s |
| Tick de 1 s | `Timer { interval: 1s; running: player.playing; triggered => { ... } }` |
| Mesh ambiente | 3 `Image` circulares pré-renderizadas (11.1) |
| Grão | `Image` de ruído com `opacity` = `gl-grain` |
| Capas | `Image` de arquivo (seção 12) com reserva gerada |

---

## 15. Acessibilidade

- Toda ação tem rótulo de leitura (`accessible-label` no Slint, correspondente aos `aria-label` do HTML).
- Áreas de toque de 48 dp (seção 2.5).
- Foco visível para navegação por teclado ou leitor de tela: contorno de 2 dp em `accent`.
- Movimento reduzido: com `prefers-reduced-motion` ou "Animações expressivas" desligado, nenhuma
  animação de posição ou escala roda; só trocas de cor, instantâneas.
- Contraste: seção 2.2.
- Escala de fonte do sistema até 130%: seção 11.3.

---

## 16. Como rodar a referência

Abra `kazzu0_4.html` no navegador (idealmente com a janela em 390 px de largura, ou pelo modo de
dispositivo do navegador). Não há dependências externas. Para avaliar a camada FX, use Chrome ou
WebView Android recente (`cqw`/`cqh`, squircle e morph dependem dele).

Para testar o encaixe do player, abra a tela em uma das alturas da tabela 7.2, toque na faixa
(ou chame `openPlayer()`) e confira se a área do player não rola.

Para testar a letra, uma faixa precisa ter `lyrics` preenchidas: use texto de sua autoria ou
licenciado. O HTML não traz letras.

---

## 17. Pendências para a versão Slint

1. Confirmar, na versão do Slint escolhida: `drop-shadow-*`, easing `cubic-bezier`, eixos de fonte variável.
2. Gerar as imagens pré-renderizadas (capa desfocada, mesh, anel, grão, reflexo) no Rust, com cache.
3. Shader de refração real (fase 2, só em Alto desempenho).
4. Capas reais com licença (seção 12).
5. Letras licenciadas com `times` por linha, para sincronia real (seção 7.4).
6. Persistir "Vidro e textura" e o modo de desempenho entre sessões (hoje não são salvos).
7. Medir quadros por segundo em aparelho real (seção 13).
8. Dados reais de Replay e biblioteca (seção 9).
9. Persistir `vinyl_mode` entre sessões (hoje volta a `disc` ao recarregar).
10. Telas de playlist e de artista (hoje os toques mostram "Em breve").
11. Chips do Início (Playlists, Músicas, Podcasts) ainda não filtram nada.
