# Mapa do Kazzu

Índice único do repositório: design, protótipos, builds, testes, capturas e documentação. Os caminhos abaixo são relativos a `docs/`.

## Design

| O que | Onde |
|---|---|
| Direção atual (v4) | [`design/v4/README.md`](../design/v4/README.md) · [`DIRECTION.md`](../design/v4/DIRECTION.md) · [`DESIGN.md`](../design/v4/DESIGN.md) |
| Tokens | [`design/v4/tokens.json`](../design/v4/tokens.json) · [`design/v4/slint/tokens.slint`](../design/v4/slint/tokens.slint) |
| Componentes Slint | [`design/v4/slint/`](../design/v4/slint/) |
| Marca, ícones e fontes | [`design/assets/`](../design/assets/) · [`design/v4/fonts/`](../design/v4/fonts/) |
| Contraste | [`design/v4/contrast-report.md`](../design/v4/contrast-report.md) · [`contrast-tokens.md`](../design/v4/contrast-tokens.md) |
| UX | [`design/v4/ux/README.md`](../design/v4/ux/README.md) |
| Legado v3 | [`design/README.md`](../design/README.md) · [`design/DESIGN.md`](../design/DESIGN.md) · [`design/slint/`](../design/slint/) · [`design/ux/`](../design/ux/) |

## Protótipos

| Tipo | Entrada |
|---|---|
| v4 navegável (atual) | [`design/v4/prototypes/index.html`](../design/v4/prototypes/index.html) |
| v3 navegável (legado) | [`design/prototypes/index.html`](../design/prototypes/index.html) |
| Desktop autocontido | [`design/experiments/kazzu-unico.html`](../design/experiments/kazzu-unico.html) |
| Prévia Slint independente | [`design/experiments/nova-ui-teste/README.md`](../design/experiments/nova-ui-teste/README.md) |
| HTML 0.5 de referência | [`design/experiments/nova-ui-teste/ui-html/kazzu 0.5.html`](../design/experiments/nova-ui-teste/ui-html/kazzu%200.5.html) |
| Port HTML arquivado | [`design/prototypes/archived/`](../design/prototypes/archived/) |

## Builds e pacotes

| O que | Onde |
|---|---|
| Índice de artefatos | [`dist/README.md`](../dist/README.md) |
| APK atual | [`dist/android/0.5.0-alpha.1/`](../dist/android/0.5.0-alpha.1/) |
| Binário Linux AArch64 | [`dist/linux/aarch64/`](../dist/linux/aarch64/) |
| Pacotes históricos | [`dist/archive/`](../dist/archive/) |
| Logs de build | [`dist/logs/frontend-0.5.0-alpha.1/`](../dist/logs/frontend-0.5.0-alpha.1/) |
| Scripts e CI | [`scripts/`](../scripts/) · [`../.github/workflows/build.yml`](../.github/workflows/build.yml) |
| Empacotamento Android | [`ANDROID.md`](ANDROID.md) · [`PACKAGING.md`](PACKAGING.md) · [`DEPENDENCIES.md`](DEPENDENCIES.md) |

## Testes e validação

| O que | Onde |
|---|---|
| Relatório da reconstrução | [`tests/TEST_REPORT.md`](tests/TEST_REPORT.md) |
| Testes Rust | [`crates/kazzu-app/tests/`](../crates/kazzu-app/tests/) · [`kazzu-audio`](../crates/kazzu-audio/tests/) · [`kazzu-library`](../crates/kazzu-library/tests/) · [`kazzu-ytm`](../crates/kazzu-ytm/tests/) · [`kazzu-ui-gallery`](../crates/kazzu-ui-gallery/tests/) |
| Limitações conhecidas | [`FRONTEND_RECONSTRUCTION/KNOWN_LIMITATIONS.md`](FRONTEND_RECONSTRUCTION/KNOWN_LIMITATIONS.md) |
| Snapshots (gerados, ignorados) | `target/ui-snapshots/` · `target/frontend-snapshots-gpu/` |
| Comando | `cargo test --workspace` (na raiz) |

## Capturas ("prints")

| Conjunto | Onde |
|---|---|
| UI v6 usada no README | [`design/v4/shots/ui-nova/`](../design/v4/shots/ui-nova/) |
| Visão geral e prova de vidro | [`key-screens.webp`](../design/v4/shots/key-screens.webp) · [`glass-proof.webp`](../design/v4/shots/glass-proof.webp) |
| Protótipos v4 renderizados | [`design/v4/shots/proto/`](../design/v4/shots/proto/) |
| Slint v4 renderizado | [`design/v4/shots/slint/`](../design/v4/shots/slint/) |
| Fluxos de UX | [`design/v4/shots/ux/`](../design/v4/shots/ux/) |
| Slint v3 (legado) | [`design/slint/v3-home-dark.png`](../design/slint/v3-home-dark.png) |

## Status e documentação

| O que | Onde |
|---|---|
| Escopo atual | [`status/IMPLEMENTATION_PLAN.md`](status/IMPLEMENTATION_PLAN.md) |
| Progresso atual | [`status/IMPLEMENTATION_PROGRESS.md`](status/IMPLEMENTATION_PROGRESS.md) |
| Índice antigo (histórico) | [`status/PROJECT_INDEX.md`](status/PROJECT_INDEX.md) |
| Arquitetura | [`ARCHITECTURE.md`](ARCHITECTURE.md) · [`FRONTEND_RECONSTRUCTION/`](FRONTEND_RECONSTRUCTION/) |
| Auditorias de UI/UX | [`ui-audit/`](ui-audit/) |
| Pesquisa | [`research/`](research/) |
| REFERENCE_MAP | [`REFERENCE_MAP.md`](REFERENCE_MAP.md) |
| Briefing e notas | [`BRIEF.md`](BRIEF.md) · [`BRIEFING-GEMINI-STITCH.md`](BRIEFING-GEMINI-STITCH.md) · [`UI_NOTES.md`](UI_NOTES.md) · [`CONTAS-E-PASTAS.md`](CONTAS-E-PASTAS.md) |
| Mapa mental | [`mindmap/Kazzu.canvas`](mindmap/Kazzu.canvas) |
| Arquivo | [`archive/README.md.bak`](archive/README.md.bak) |

## Estrutura do código

`../crates/` (workspace Rust), `../android/` (nativo), `../scripts/` (build/validação) e `../target/` (gerado pelo Cargo) permanecem na raiz: mudanças nesses caminhos exigem atualizar `.github/workflows/build.yml` e os scripts de build.
