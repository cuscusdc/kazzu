# Índice do projeto Kazzu

## Aplicativo

- `crates/` — crates Rust do app, player, biblioteca, provedores e galeria.
- `android/` — recursos nativos Android.
- `design/` — fontes, protótipos, ferramentas e definições visuais.
- `scripts/` — compilação, empacotamento e validação.

## Documentação

- `docs/` — guias do projeto.
- `docs/FRONTEND_RECONSTRUCTION/` — arquitetura, contratos, limitações e relatório da reconstrução.
- `UI_UX_AUDIT/` — auditoria de interface e UX.

## Pacotes e evidências

- `dist/README.md` — índice de APK, AppImage e binários organizados por plataforma e versão.
- `dist/android/` e `dist/linux/` — artefatos desta reconstrução.
- `dist/archive/` — pacotes históricos preservados.
- `dist/logs/` — logs selecionados da reconstrução.
- `target/` — área de compilação Cargo, APKs de teste e capturas. É gerada pelas ferramentas de build.

Os arquivos de configuração e automação existentes na raiz foram mantidos no lugar.

## Port do HTML fornecido

- `design/prototypes/archived/kazzu-html-player/` — port isolado arquivado. A interface ativa pertence a `crates/kazzu-app/`.
- `design/prototypes/archived/kazzu-html-player/docs/INVENTORY.md` — inventário do material de referência.
