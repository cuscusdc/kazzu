# Kazzu no Android

A UI v6 de produção é composta em `kazzu-app/src/android_app.rs`. O Rust controla o áudio, a fila, o SQLite e os provedores. O global Slint `AppBridge` recebe os modelos e encaminha os comandos. A v5 continua sendo o ponto de entrada visual desktop. O estado de implementação e as validações pendentes estão em [IMPLEMENTATION_PROGRESS.md](status/IMPLEMENTATION_PROGRESS.md).

| Componente | Integração |
|---|---|
| Atividade | `KazzuActivity` estende `NativeActivity`. A ponte Java recebe os resultados do seletor de documentos, integra foco de áudio e hospeda as funções do sistema que precisam de uma classe Android. |
| Acessibilidade | `accessibility.rs` projeta os modelos do `AppBridge` numa árvore com IDs estáveis; `KazzuActivity` instala um `AccessibilityNodeProvider` no decor view e encaminha cliques, incrementos e remoção da fila ao backend. TalkBack ainda exige validação no aparelho. |
| Ciclo de vida | O motor e a qualidade remota sobrevivem à recriação da Activity; cada geração recebe um clone do handle e reidrata o `AppBridge` pelo snapshot do motor. Mudanças de configuração preservam `KazzuMediaService` e o foco de áudio; o encerramento real chama `nativeShutdown`. |
| Player | CPAL e motor compartilhado. A fila contém entradas locais e remotas, com resolução remota no momento de abrir ou preparar a faixa seguinte. |
| Segundo plano | `KazzuMediaService` publica estado e controles por `MediaSession` e notificação `MediaStyle`. Apoia playback ao esconder ou bloquear a tela enquanto a Activity e o processo estão vivos. |
| Persistência | `getFilesDir()` contém SQLite, configuração, preferências, biblioteca YTM e cópias importadas. As capas escolhidas pelo usuário são persistidas com suas playlists. |
| Cache | `getCacheDir()` contém capas recriáveis, metadados YTM e streams completos sujeitos à política LRU do motor. |
| Arquivos locais | SAF concede leitura persistente aos diretórios escolhidos. Um importador copia arquivos com substituição atômica para a biblioteca privada e reutiliza o scanner existente. Edição de tags se aplica às cópias. |
| Provedores | YouTube Music visitante para reprodução. Importação OAuth de coleções usa os conectores existentes e descarta o token depois da solicitação. |
| Privacidade | Histórico e Replay são locais. Last.fm envia scrobbles somente após a autorização explícita, com a sessão protegida pelo Android Keystore. Controle e salas LAN são iniciados pelo usuário, protegidos por código efêmero e expirados após até 30 minutos. |
| Empacotamento | O script existente usa `javac` + `d8` para a ponte Java e mantém `aapt2`, `zipalign` e `apksigner`. SDK mínimo 26 e target 34 no manifesto. |

O vídeo usa o [player oficial do YouTube](https://developers.google.com/youtube/terms/required-minimum-functionality), com opção de abrir externamente quando a incorporação estiver indisponível. A integração [Last.fm](https://www.last.fm/api/authspec) requer uma chave e um segredo válidos antes da autorização no navegador. Em Linux e macOS, KAZZU_DISCORD_APPLICATION_ID habilita presença pelo [IPC local do Discord](https://docs.discord.com/developers/topics/rpc) quando o cliente está em execução.

O manifesto pede Internet para provedores e LAN e as permissões para o serviço de mídia. O SAF concede acesso aos arquivos selecionados pelo usuário. Logs do aplicativo permanecem no logcat.

## Ambiente de build (o que foi instalado)

Tudo em `/home/sacer/.local/share/android-sdk` (`ANDROID_HOME`), baixado de `dl.google.com`, sem custo. Instalação, a partir do zero:

```bash
export ANDROID_HOME=$HOME/.local/share/android-sdk
mkdir -p $ANDROID_HOME/cmdline-tools && cd /tmp
curl -O https://dl.google.com/android/repository/commandlinetools-linux-16111833_latest.zip   # sha1 e025545c62a8e64c7559119566a569fb1dec5f60
unzip commandlinetools-linux-16111833_latest.zip && mv cmdline-tools $ANDROID_HOME/cmdline-tools/latest
yes | $ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager --sdk_root=$ANDROID_HOME --licenses
$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager --sdk_root=$ANDROID_HOME \
  "platform-tools" "platforms;android-35" "build-tools;35.0.1" "ndk;30.0.16248370" "cmake;3.31.6"
# opcional, para testar sem aparelho:
$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager --sdk_root=$ANDROID_HOME "emulator" "system-images;android-34;default;x86_64"
rustup target add aarch64-linux-android x86_64-linux-android armv7-linux-androideabi
```

- **NDK r30 (30.0.16248370) é obrigatório.** O Skia pré-compilado que o `skia-bindings` baixa foi compilado com o libc++ do r30; com o NDK r28 ou r29 o APK compila, mas **não abre** (`dlopen failed: cannot locate symbol "_ZNSt6__ndk113__hash_memoryEPKvm"`). Foi o que aconteceu na primeira tentativa.
- JDK: serve o do sistema (testado com **OpenJDK 25** localmente e 21 no CI). Com SDK build-tools 35+ o `d8` aceita JDK 21+; com build-tools antigas, usar JDK 17.
- Android Studio **não** é necessário. O `rustup` do sistema deste computador (`/usr/lib/rustup`) deixou instalar os alvos Android normalmente (toolchain em `/root/.rustup`).
- Licenças do SDK: aceitas com `sdkmanager --licenses`, como o autor pediu; os textos estão em `$ANDROID_HOME/licenses`.

## Compilar o APK

```bash
export ANDROID_HOME=$HOME/.local/share/android-sdk
export CARGO_TARGET_DIR=$PWD/target/android      # (padrão do script) não disputa com os builds do desktop
scripts/build-apk.sh                              # arm64-v8a, assinado com a chave de release
scripts/build-apk.sh --abi x86_64 --debug-sign    # para o emulador, com chave descartável
scripts/build-apk.sh --version 0.1.0-alpha.2
# saída: target/android/apk/kazzu-<versão>-<abi>.apk  e  .apk.sha256
```

O script define o `CC`/linker do NDK por ABI, roda `cargo rustc -p kazzu-app --lib --release --no-default-features --crate-type cdylib --target <alvo>` (sem a feature `demo`), remove símbolos, empacota com `aapt2` (manifesto + ícones de `android/res`), grava `libkazzu_app.so` **sem compressão e alinhada a 16 KB** (exigência do Android 15+ em aparelhos com página de 16 KB; o NDK r28+ já liga com esse alinhamento), roda `zipalign` e assina (esquemas v2/v3) com `apksigner`. O `versionCode` vem da versão (`0.1.0-alpha.1` → 10001; versão final tem número maior que a alpha).

Os ícones (`android/res/**`) são gerados de `design/assets/` por `android/gen-icons.sh` e versionados; ícone adaptativo (marca "K" sobre fundo escuro) + ícone monocromático (temas do Android 13+) + PNG legado.

## Assinatura e backup da chave (**leia**)

O APK de release é assinado com uma chave **nova** gerada para o Kazzu:

- keystore: `~/.android/kazzu-release.jks` (PKCS12, RSA 4096, alias `kazzu`, validade de ~100 anos)
- senha: `~/.android/kazzu-release.pass` (arquivo `chmod 600`, fora do git; o script nunca a imprime)

**Essa chave é insubstituível.** O Android só deixa atualizar um app por cima se o novo APK tiver a **mesma** assinatura: perdendo a keystore, ninguém consegue atualizar o Kazzu instalado (só desinstalar e instalar de novo, perdendo os dados). Nunca vai para o git (nem para este repositório, mesmo privado). **Faça backup agora**, por exemplo:

1. copie os dois arquivos para um pendrive/HD externo e/ou para um gerenciador de senhas (anexo seguro, ex.: KeePassXC, gratuito);
2. guarde uma cópia em outro lugar que não seja este SSD (se o SSD falhar, a chave some junto);
3. opcional para o CI: `base64 -w0 ~/.android/kazzu-release.jks | gh secret set KAZZU_KEYSTORE_B64 -R cuscusdc/kazzu` e `gh secret set KAZZU_KEYSTORE_PASSWORD -R cuscusdc/kazzu < ~/.android/kazzu-release.pass` (o GitHub guarda cifrado e nunca mostra de volta; não substitui o backup).

Para conferir a impressão digital da chave (sem expor a senha): `keytool -list -v -keystore ~/.android/kazzu-release.jks -storepass:file ~/.android/kazzu-release.pass | grep SHA256`.

Para fixar o certificado oficial, configure KAZZU_SIGNER_SHA256 no ambiente local ou nas variáveis do repositório usadas pelo workflow. O script compara a impressão digital do APK com o valor esperado em builds oficiais.

## CI (GitHub Actions, gratuito)

`.github/workflows/build.yml`:

- push de tag `v*` → compila, assina e anexa `kazzu-<versão>-arm64-v8a.apk` + `.sha256` à release da tag (prerelease se a tag tiver `-`);
- pull request / execução manual → só compila (artefato do workflow, chave descartável).

Sem os segredos `KAZZU_KEYSTORE_B64` e `KAZZU_KEYSTORE_PASSWORD` o CI assina com chave descartável e **não publica**, de propósito (APK com outra chave não atualiza o instalado). Repositório privado usa os minutos do plano gratuito (2000/mês); um build leva ~10–15 min com cache.

## Publicar uma versão

```bash
scripts/build-apk.sh --version 0.1.0-alpha.2
gh release create v0.1.0-alpha.2 --prerelease --title "Kazzu 0.1.0-alpha.2" --notes-file NOTAS.md \
   target/android/apk/kazzu-0.1.0-alpha.2-arm64-v8a.apk target/android/apk/kazzu-0.1.0-alpha.2-arm64-v8a.apk.sha256
```

(ou só empurrar a tag e deixar o CI fazer, com os segredos cadastrados). O repositório é **privado**: o link da release só abre logado numa conta com acesso (no celular, entre no GitHub pelo navegador antes de baixar o APK).

## Verificação histórica (v0.1.0-alpha.1)

**Emulador** (AVD `kazzu34`: Android 14 / API 34, imagem AOSP x86_64, KVM, GPU por software `swiftshader_indirect`), com o APK `x86_64` (mesmo código do arm64):

- instala, abre e renderiza a UI com Skia (vidro/blur incluso); captura de tela da tela inicial e do estado com músicas feita via `adb exec-out screencap`;
- pede a permissão "acessar músicas e áudio" na primeira execução; ao permitir (evento `Resume`), indexa `/sdcard/Music` (2 álbuns FLAC enviados por `adb push`: 7 faixas, tags e formato "FLAC 16/44,1" corretos);
- reprodução: o stream AAudio abre e inicia (`AAudioStreamBuilder_openStream() returns 0 = AAUDIO_OK`), a barra de progresso e o mini-player avançam. **Não se ouviu o som** (emulador sem áudio, `-no-audio`);
- ícone adaptativo aparece corretamente na tela de recentes.

**Estático, no APK `arm64-v8a` final** (`kazzu-0.1.0-alpha.1-arm64-v8a.apk`, 25.225.519 bytes):

- `aapt2 dump badging`: pacote `app.kazzu.player`, `versionName 0.1.0-alpha.1`, `minSdk 26`, `targetSdk 34`, `native-code: arm64-v8a`, permissões **só** `INTERNET`, `READ_MEDIA_AUDIO`, `READ_EXTERNAL_STORAGE (maxSdk 32)`;
- `apksigner verify`: assinatura v2 e v3 válidas, 1 signatário (`CN=Kazzu`, RSA 4096, certificado SHA-256 `6358e521…2e92`);
- `zipalign -c -P 16`: OK; `llvm-readelf`: `AArch64`, segmentos `LOAD` com alinhamento 0x4000 (16 KB), `NEEDED` só `liblog, libandroid, libaaudio, libdl, libEGL, libGLESv2, libm, libc` (libc++ e libopus estáticas dentro da lib), símbolos `android_main`/`ANativeActivity_onCreate` exportados, nenhum símbolo `__hash_memory` indefinido;
- o desktop continua compilando: `cargo check -p kazzu-app --all-targets` (Linux) passa com as mudanças de `Cargo.toml`/`cfg`.

**NÃO testado:** aparelho físico (nenhum, ARM ou não); o APK arm64 em si nunca foi executado (o emulador x86_64 não roda código arm64); som real, fones/Bluetooth, rotação, bloqueio de tela, Android 15+ (páginas de 16 KB), desempenho do blur em GPU real.

**Problemas da alpha.1 e o que foi feito na v0.2:**
- desenhava **por baixo** das barras do sistema: a UI v2 usa `safe-area-insets` (o backend Android do Slint os preenche sozinho) e tem fade superior/inferior; verificado no emulador;
- capas em branco: **causa encontrada**, não era render. Com a permissão só de áudio o Android não deixa ler `cover.jpg` da pasta; capas embutidas na tag aparecem. Capa ausente agora mostra o placeholder `disc`. Pedir `READ_MEDIA_IMAGES` fica como decisão do autor;
- "Voltar" não fechava player/detalhe: agora o `FocusScope` raiz recebe `Key.Back` e chama `go-back()` (fecha folha → fila → player → detalhe → volta ao Início → sai).

## Verificação histórica (v0.2.0-alpha.3, emulador x86_64)

Emulador AVD `kazzu34` (Android 14, KVM, `-gpu swiftshader_indirect`, 1080×2400, navegação por 3 botões), APK `--debug-sign`:

- abre e o **Início do YouTube Music carrega** sozinho (rede do emulador funciona): banner, chips, destaques com capas reais, escolhas rápidas;
- **tocar uma faixa remota**: toque na linha → resolve → stream HTTP → `AAudioStreamBuilder_openStream() returns 0 = AAUDIO_OK` → mini player e progresso andando; player com selo `OPUS 48 kHz`/`Do YouTube Music`/"Original, sem equalizador" (ajustado depois: o selo agora usa o formato que o provedor informou);
- **safe areas**: wordmark abaixo da barra de status; barra inferior acima dos botões do sistema;
- **Voltar**: fecha o player, fecha o detalhe de playlist, no Início sai do app;
- biblioteca local (`adb push` em `/sdcard/Music`): indexa; capa embutida aparece, `cover.jpg` não (permissão).
- **Não verificado**: som real (emulador sem áudio), aparelho físico (ARM), TalkBack, teclado sobre a folha Nova playlist, desempenho do blur em GPU real, segundo plano/tela apagada.

## Validação da integração atual

A revisão desta implementação foi exclusivamente estática. Antes de atribuir aceite funcional, valide o bootstrap e a v6 no aparelho, a navegação do TalkBack pela árvore publicada, os controles e a remoção da fila por leitor de tela, a troca de faixa e de rota de áudio, a recriação da Activity em rotação com reprodução ativa, o bloqueio de tela, a retomada do processo, a reprodução offline já armazenada, a entrada do SAF e a restauração das permissões. Valide também a assinatura com credenciais reais e a compatibilidade do desktop.

O escopo restante e o ponto de retomada estão em [IMPLEMENTATION_PROGRESS.md](status/IMPLEMENTATION_PROGRESS.md). Os resultados históricos acima se referem aos APKs indicados nas respectivas seções.

### Mudanças recentes (consolidação pré-compilação)

- Scrobble Last.fm corrigido para faixas locais e remotas (ordem `TrackEnded`/`TrackChanged` não impede mais o envio).
- `onDestroy` da Activity preserva engine, executores e progresso em recriações (rotação); `nativeShutdown`/`stopService` só quando realmente destrói.
- Backup v2 exporta/importa curtidas e histórico além de playlists, preferências e capas; credenciais e áudio continuam de fora.
- Botões Compartilhar e Vídeo do player conectados a `AppBridge.share-now()`/`watch-now()` (funcionais, sem toast).
- Versão exibida sincronizada com `Cargo.toml` (`0.6.0`).
- `crates/kazzu-app/Cargo.toml` declara `[lib] crate-type = ["cdylib"]`, produzindo `libkazzu_app.so` no Android.

### Mudanças desta rodada (última estática antes da compilação)

- `browse_folders` deixa de ser stub: abrir/remover/escanear pastas locais via SAF estão completos (`remove_folder` e `rescan` já existiam).
- Canais de eventos do motor aumentados (`EVENT_CAPACITY` 4096→16384, `STREAM_EVENT_CAPACITY` 256→1024) sem bloquear a thread de áudio.
- Placeholders `comingSoon` do índice de Ajustes substituídos por rows informativas (Last.fm, Discord, Criar sala, Entrar com código), sem simular funcionamento.
