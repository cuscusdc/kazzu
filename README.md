# Kazzu

## Sobre

Kazzu é um player de música livre em Rust e Slint, voltado a Hi-Fi e Hi-Res até 24 bits, privacidade extrema e nenhuma telemetria. A interface segue uma linguagem Apple-like, com vidro, temas claro e escuro e ajustes de aparência. O núcleo prioriza baixo overhead, com callback de áudio sem alocação e renderer de software como alternativa para máquinas modestas.

Mapa de design, protótipos, builds, testes e capturas: [docs/INDEX.md](docs/INDEX.md).

## Capturas de tela

<p align="center">
  <img src="design/v4/shots/ui-nova/01-home.png" width="72" alt="Início">
  <img src="design/v4/shots/ui-nova/02-explore.png" width="72" alt="Explorar">
  <img src="design/v4/shots/ui-nova/03-library.png" width="72" alt="Biblioteca">
  <img src="design/v4/shots/ui-nova/04-search.png" width="72" alt="Busca">
  <img src="design/v4/shots/ui-nova/05-replay.png" width="72" alt="Retrospectiva">
  <img src="design/v4/shots/ui-nova/06-settings.png" width="72" alt="Configurações">
  <img src="design/v4/shots/ui-nova/07-settings-page.png" width="72" alt="Detalhes de configuração">
  <img src="design/v4/shots/ui-nova/08-appearance.png" width="72" alt="Aparência">
  <img src="design/v4/shots/ui-nova/09-light.png" width="72" alt="Tema claro">
  <img src="design/v4/shots/ui-nova/10-player.png" width="72" alt="Player">
  <img src="design/v4/shots/ui-nova/11-queue.png" width="72" alt="Fila">
  <img src="design/v4/shots/ui-nova/12-more.png" width="72" alt="Mais opções">
  <img src="design/v4/shots/ui-nova/13-lyrics.png" width="72" alt="Letras">
</p>

## Funcionalidades atuais

- Reprodução de arquivos locais em FLAC, WAV, AIFF, CAF, ALAC, MP3, AAC, Vorbis, Opus e ADPCM, com saída bit-perfect quando o volume é 1 e o dispositivo aceita a taxa original.
- Gapless entre faixas do mesmo formato, seek por amostra e reprodução contínua durante streaming HTTP com Range, retomada e cache em disco.
- Fila com ordem, tocar a seguir, mover, remover, shuffle e repeat.
- Biblioteca local em SQLite com varredura incremental, leitura de tags, capas, busca por texto, playlists, curtidas e histórico.
- YouTube Music sem login: início, busca, álbuns, artistas, playlists e rádio, com resolução de stream e cache de metadados.
- Configurações persistentes em TOML: tema, acento, volume, qualidade de stream, movimento e transparência.
- Interface desktop com navegação por teclado e fallback para renderer de software.
- No Android, a versão publicada indexa a pasta Music e reproduz arquivos locais.

## Roadmap

Em desenvolvimento: integração completa do motor e da biblioteca no APK, seletor de pastas via SAF, notificações e MediaSession.

Planejado: downloads para uso offline, equalizador, crossfade, seleção de saída de áudio, timer de sono, letras sincronizadas, reprodução de vídeo, integrações externas e sincronização entre dispositivos.

## Instalação

Android ARM64, Android 8 ou superior: [Instalar no Android](https://github.com/cuscusdc/kazzu/releases/download/v0.6.0-ci.3/kazzu-0.6.0-ci.3-arm64-v8a.apk)

O link aponta para o APK da pré-release de CI mais recente, `v0.6.0-ci.3`. A build usa chave descartável e apresenta a interface v6 em modo demonstração; desinstale a versão anterior antes de instalar. URLs de assets por tag são permanentes e mantêm a versão no nome do arquivo.

Para desktop, compile a partir do fonte com os comandos abaixo.

## Desenvolvimento

Stack: Rust 2024, Slint 1.19, SQLite com FTS5, symphonia e libopus, cpal, ureq com rustls e TOML. O projeto não usa Tokio; a comunicação entre threads passa por canais e um ring buffer SPSC.

O workspace separa `kazzu-core` (tipos), `kazzu-audio` (motor e decodificação), `kazzu-library` (banco, varredura e buscas), `kazzu-ytm` (provedor YouTube Music) e `kazzu-app` (interface, controller, configuração e integração). A UI Slint envia comandos ao controller, que coordena o worker e o motor; o callback de áudio apenas consome o ring buffer.

```sh
cargo run -p kazzu-app --release
cargo run -p kazzu-app --release -- --demo-library target/fixtures/kazzu-demo
cargo run -p kazzu-app --release -- --no-youtube
cargo test --workspace
```

Para Android, `scripts/build-apk.sh --abi arm64-v8a --version 0.6.0` gera o APK sem Gradle, usando Android SDK/NDK r30 e a chave de release configurada em `KAZZU_KEYSTORE` e `KAZZU_KEYSTORE_PASS_FILE`.

## Licença

O código do workspace é declarado como MIT no `Cargo.toml`. A distribuição que inclui Slint sob a opção GPL-3.0 fica sujeita à GPL-3.0; o repositório ainda não traz um arquivo `LICENSE` com a decisão formal.

## Monolith Studios

O Kazzu é desenvolvido pela Monolith Studios.
