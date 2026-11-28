# design/

Design system e planejamento de UX do Kazzu (**v3: Apple-like, fundo neutro, vidro claro sobre conteúdo**). Comece por `DESIGN.md` (como é) e `ux/UX_PLAN.md` (o quê e por quê; **aprovado e inalterado**: a v3 só muda o visual). O implementador de UI segue `SLINT_SPEC.md`.

## Mapa do design

| Área | Entrada |
|---|---|
| Direção atual (v4) | `v4/README.md`, `v4/DIRECTION.md`, `v4/DESIGN.md` |
| Protótipos atuais | `v4/prototypes/index.html` |
| Capturas atuais | `v4/shots/ui-nova/`, `v4/shots/proto/`, `v4/shots/slint/`, `v4/shots/ux/` |
| Slint atual | `v4/slint/` |
| v3 (legado) | `DESIGN.md`, `SLINT_SPEC.md`, `prototypes/`, `slint/`, `ux/` |
| Experimentos | `experiments/nova-ui-teste/` (prévia Slint v6) e `experiments/kazzu-unico.html` (protótipo desktop autocontido) |
| Índice geral do repositório | `../docs/INDEX.md` |

Abaixo, a documentação operacional da v3, mantida porque ainda descreve os comandos de geração e verificação.

| Pasta/arquivo | Conteúdo |
|---|---|
| `DESIGN.md` | direção de arte v3: cor neutra, vidro em 3 materiais, tipografia Inter, forma, antes × depois |
| `SLINT_SPEC.md` | árvore de componentes, medidas, estados; **§0.1 = diferenças para o código atual do app** |
| `tokens.json` | fonte única dos tokens |
| `slint/` | `tokens.slint` (gerado), `atoms.slint`, `chrome.slint`, `app-contract.slint`, `preview.slint`, `assets/blur-*.png` (capas pré-desfocadas do player) e **renders reais** `v3-*.png` (FemtoVG com backdrop-blur; `v3-home-software-dark.png` = renderer software, fallback sólido) |
| `prototypes/` | HTML navegável mobile-first (abra `prototypes/index.html`); **`glass-proof.html` = prova de vidro**; `assets/before-after.png` = painel antes × depois |
| `ux/` | plano de UX e `contrast-report.md` (contraste medido nas capturas v3) |
| `assets/` | logo, ícone do app, favicons (**inalterados**) e 57 ícones (`icons/`; v3 acrescenta `home-fill`, `explore-fill`, `library-fill`) |
| `fonts/` | Inter (400/500/600/700) e Inter Display (600/700), Geist Mono (400/500): OFL 1.1, com as licenças |
| `tools/` | geradores e verificadores (abaixo); `slint-shot/` renderiza o Slint de verdade |

## Regenerar e verificar

```
python3 tools/gen_tokens.py        # tokens.slint + tokens.css + conferência AA dos pares (inclui vidro)
python3 tools/make_assets.py       # ícones, capas fictícias, sprite. ATENÇÃO: também reescreve logos/favicons a partir dos tokens;
                                   # a marca é inalterada: depois rode `git checkout -- assets/favicon.* assets/icon-* assets/kazzu-*`
python3 tools/build_prototypes.py  # protótipos HTML
python3 tools/check_render.py --out ux/contrast-report.md   # contraste real em ~117 capturas (Chromium + Node >= 22 + Pillow)
node tools/capture.mjs home 390 844 /tmp/home.png "theme=dark"   # uma captura (viewport exato, via CDP; apaga o perfil temporário)

# Slint de verdade (FemtoVG, blur real) — precisa de Xvfb. Use um CARGO_TARGET_DIR fora do workspace e APAGUE depois (~1 GB):
cd tools/slint-shot && CARGO_TARGET_DIR=../../../target/design-v3 cargo build --offline --release
xvfb-run -a -s "-screen 0 1600x1200x24 +extension GLX" ../../../target/design-v3/release/kazzu-slint-shot \
  --shot home,390x844,dark,0,0,../../slint/v3-home-dark.png          # cena,LxA,tema,reduzir-transparência,rolagem,arquivo
# cenas: home home-playing home-wide library settings player player-wide sheet proof; `--renderer software` = fallback sólido
```

Opções dos protótipos (hash na URL): `#theme=light|dark`, `#rt=1` (reduzir transparência), `#sat=0` (sem `saturate`, como no Slint), `#c=01..10` (capa), `#s=` (estado), `#scroll=N` (rola N px: mostra o cabeçalho de vidro e o conteúdo sob o cromo).

## O que foi rodado na v3 (e o resultado)

- **`gen_tokens.py`**: todos os pares AA passam (texto, acento, vidro 1·2·3 sobre fundo, conteúdo médio e "branco sob o fade", sólidos, player com capa branca).
- **Slint 1.19**: `tokens.slint`, `atoms.slint`, `chrome.slint` e `preview.slint` **compilam** e foram **renderizados com FemtoVG** (backdrop-blur real, via Xvfb/Mesa) em 390×844 e 1280×800, claro e escuro, com e sem transparência e no renderer software. O vidro aparece (capas desfocadas atrás da barra, do mini player, do cabeçalho de rolagem, da legenda do cartão e da folha) e a borda em gradiente é desenhada.
- **Protótipos HTML**: 390×844, 360×740, 768×1024 e 1280×800, claro e escuro, com e sem transparência, com rolagem; contraste medido em captura real (`ux/contrast-report.md`): **2.122 textos, 0 abaixo de AA, menor 4,83:1**.
- **`hallmark` (`sloplint.mjs` nos protótipos)**: corrigidos o dash no `<title>`, cores literais fora de tokens (agora `--media-*`), `100vw`, `1fr` nu, `transition: height`, polegar escondido em repouso e o carimbo na linha 1 do CSS. **Restam**: (a) os 10 blocos `.cv-NN` de `covers.css` (dados gerados por capa, usados só pelo acento dinâmico opcional) contam como "cor fora de token"; (b) os avisos **7a/7b "pure #000/#fff"**: preto e branco verdadeiros são **decisão do autor** (registrada como waiver no carimbo); o linter não aceitou o waiver e continua reportando como REFLEX (não é FAIL de Floor). O hallmark é de sites: os gates de hero/seções/Chrome não se aplicam a um app e não foram rodados.
- **`taste-skill`**: lido por inteiro; é um gate de sites (Node/Chrome, deals, `gate.mjs`), não rodado. Princípios aplicados: sem caption sob imagem, hover/pressão visíveis, bordas iguais, ícones reais, fotos/capas reais em vez de ilustração vazia, uma seção por tela.
- **Não verificado**: Skia, desempenho do blur em GPU fraca/Android real, TalkBack, testes com pessoas; não rodamos `cargo test/clippy` do workspace (fora do escopo e do orçamento de disco).
