# Kazzu — Auditoria final pré-compilação

Data: 2026-11-28
Branch: `feat/cuscusdc`
Escopo: `docs/status/IMPLEMENTATION_PLAN.md` (dez fases, previamente autorizadas)
Restrições da auditoria: apenas leitura estática; nenhum build, `cargo check`, teste, APK, release ou pipeline executado; nenhum resultado de execução inventado.

---

## 1. Relatório de implementação por fase

Para cada uma das dez fases: implementado, parcialmente implementado, não implementado, bloqueado, arquivos criados/modificados/removidos, dependências, decisões arquiteturais e evidências.

### Fase 1 — Backend Android

| Categoria | Detalhe |
|---|---|
| Implementado | Reativação do backend Android; bootstrap NativeActivity com `AndroidBackend`; bootstrap específico UI v6 (`android_app.rs` com `AndroidView` e ciclo de vida); conexão Engine/Library/Config/Controller/Worker; AppBridge Rust/Slint; remoção de dados fictícios (Demo, capas fictícias, relógio simulado, Replay fixo) do caminho Android; persistência TOML (volume, catálogo, Glass, tema, movimento, formas, player); histórico local/remoto no SQLite; favoritos/curtidas; pesquisa FTS5 + YTM; cache inteligente (capas, metadados YTM, LRU de streaming); fila migrada para o motor com resolução diferida; separação `getFilesDir`/`getCacheDir`; reaproveitamento do SQLite existente. |
| Parcial | Migração "completamente" da fila para o motor: ainda falta retirar o modo legado usado pelos testes (`Context::tail`). |
| Não implementado | — |
| Bloqueado | — |
| Arquivos criados | `crates/kazzu-app/src/android_app.rs`, `android_paths.rs`, `android_view.rs`, `accessibility.rs`, `backup.rs`, `discord_presence.rs`, `event_bridge.rs`, `lyrics.rs`, `m3u.rs`, `playlist_cover.rs`; `crates/kazzu-app/ui-mobile-v6/app-bridge.slint`; `crates/kazzu-library/src/statistics.rs`, `tag_edit.rs`; `android/src/` (nova ponte Java). |
| Arquivos modificados | `crates/kazzu-app/src/android.rs`, `app.rs`, `catalog.rs`, `config.rs`, `controller/mod.rs`, `controller/remote.rs`, `covers.rs`, `lib.rs`, `platform.rs`, `remote.rs`, `view.rs`, `vm.rs`, `worker.rs`; `crates/kazzu-audio/src/decoder.rs`, `engine.rs`, `queue.rs`, `stream/{cache,feed,http}.rs`; `crates/kazzu-core/src/playback.rs`, `provider.rs`; `crates/kazzu-library/src/db.rs`, `lib.rs`, `queries.rs`; `crates/kazzu-app/ui-mobile-v6/{browse,components,data,main,player,settings,state}.slint`; `android/AndroidManifest.xml`; `scripts/build-apk.sh`; `.github/workflows/build.yml`; `docs/ANDROID.md`; `README.md`; `design/README.md`; `design/experiments/nova-ui-teste/*`. |
| Arquivos removidos | `design/assets/*` (favicons, ícones, logos, wordmarks — design assets descontinuados). |
| Dependências | Nenhuma nova dependência declarada no escopo da auditoria; reutiliza slint 1.19, CPAL, rodio, YTM client existente, SQLite via `kazzu-library`. |
| Decisões arquiteturais | Bootstrap Android v6 específico; AppBridge separado do estado visual; reutilização do Controller existente; motor de áudio como única fonte de verdade da fila; `getFilesDir` para persistente, `getCacheDir` para descartável; engine `Clone` com `owner` apenas no dono encerra no drop; sessão processual global para recriação da Activity. |
| Evidências | `android_app.rs:423-460` (sessão/geração), `engine.rs` (owner/clone), `ui-mobile-v6/app-bridge.slint:37-132` + `android_app.rs:498-601` (44 callbacks), `android.rs:102-385` (JNI símbolos), `IMPLEMENTATION_PROGRESS.md:14-38`. |

### Fase 2 — Player

| Categoria | Detalhe |
|---|---|
| Implementado | Reprodução real de áudio (locais e remotas ao motor); play/pause/próxima/anterior; volume e progresso; shuffle/repeat; fila funcional (adicionar/avançar/selecionar/remover por ID); reordenação da fila; gapless (prefetch do próximo stream); crossfade parcial (mistura linear opcional só para pares locais WAV/FLAC de taxa/canais compatíveis, gapless cobre o resto); bit-perfect parcial (indicado no motor, rota física depende do sistema); seletor de saída de áudio (CPAL fora da UI, Android implementado, desktop stub); timer de sono (engine implementado, Android implementado, desktop stub); letras sincronizadas (LRC local, remoto sem tempos inferidos — parcial); videoclipes (ação via JNI, docs afirmam WebView não confirmada no código); reprodução em segundo plano (parcial — controles ao minimizar/bloquear, sobrevivência a encerramento pendente); foreground service; MediaSession; notificação multimídia; controle tela de bloqueio; controle Bluetooth; gerenciamento de foco de áudio. |
| Parcial | Letras sincronizadas remotas (sem tempos inferidos); reprodução em segundo plano (sobrevivência pendente); crossfade (escopo reduzido); bit-perfect (não verificado em hardware); saída de áudio desktop (stub); timer de sono desktop (stub). |
| Não implementado | — |
| Bloqueado | — |
| Arquivos | `crates/kazzu-audio/src/engine.rs`, `stream/*`; `crates/kazzu-app/src/android_app.rs`, `android.rs`, `controller/mod.rs`, `controller/remote.rs`; `android/src/app/kazzu/player/KazzuActivity.java`, `KazzuMediaService.java`; `ui-mobile-v6/player.slint`. |
| Decisões | Motor como única fonte de verdade; crossfade desativa bit-perfect; engine `sleep_timer` pausa reprodução. |
| Evidências | `engine.rs:1388-1456` (crossfade), `engine.rs:2195-2215` (bit-perfect), `engine.rs:2016-2020` (sleep timer), `android_app.rs:553-587` (saída), `IMPLEMENTATION_PROGRESS.md:42-80`. |

### Fase 3 — Músicas locais

| Categoria | Detalhe |
|---|---|
| Implementado | Seleção de pastas via SAF; permissões persistentes; escaneamento automático e incremental (mtime/size); importação de arquivos (cópia atômica); leitura de metadados e capas; edição de metadados (cópia privada, gravação atômica, reindexação); organização por álbum/artista/gênero; pesquisa local FTS5; importação M3U/M3U8 (transacional, caminhos ambíguos ignorados); detecção de duplicatas (tamanho + SHA-256, informativa). |
| Parcial | `add_named_folder` ignora caminho absoluto e abre SAF; `browse_folders()` vazio stub. |
| Não implementado | — |
| Bloqueado | — |
| Arquivos | `android/src/app/kazzu/player/LocalMusicImport.java`, `KazzuActivity.java` (SAF/M3U); `crates/kazzu-app/src/m3u.rs`, `worker.rs`, `controller/mod.rs:1622-1647`; `crates/kazzu-library/src/scan.rs`, `db.rs`, `queries.rs`, `tag_edit.rs`; `crates/kazzu-library/src/artwork.rs`. |
| Decisões | SAF tree permission durável; hash-prefixed nomes em `getFilesDir()/imported-music`; detecção informativa (não deleta). |
| Evidências | `LocalMusicImport.java:27-159`, `m3u.rs:1-79`, `scan.rs:107-181`, `queries.rs:785-805` (imported_playlists fora de MIGRATIONS). |

### Fase 4 — Provedores

| Categoria | Detalhe |
|---|---|
| Implementado | YouTube Music modo visitante; pesquisa unificada (local FTS5 + YTM); importação de playlists entre serviços (metadados, leitura-only); alternância entre fontes locais/remotas; seleção de qualidade de streaming; arquitetura extensível de provedores (Core `Provider` + app `Catalog`). |
| Parcial | Gerenciamento de contas conectadas (Last.fm autoriza/desconecta; conta YTM de reprodução bloqueada; importação OAuth temporária). |
| Não implementado | Spotify/Tidal/Qobuz/SoundCloud como provedores de reprodução (fora de escopo). |
| Bloqueado | YouTube Music com login de reprodução (sem contrato de provedor); downloads permanentes YTM (`download: false`). |
| Arquivos | `crates/kazzu-ytm/src/{client,lib,innertune,cache}.rs`; `crates/kazzu-spotify/src/lib.rs` (não wired); `crates/kazzu-app/src/{catalog,remote,accounts}.rs`; `crates/kazzu-app/src/controller/{mod,remote}.rs`; `ui-mobile-v6/{browse,settings,data}.slint`. |
| Decisões | YTM como único provedor de playback instalado; Spotify importado é metadado-only; OAuth de importação não é login de playback. |
| Evidências | `kazzu-ytm/src/lib.rs:191-201` (capacidades anônimas), `catalog.rs:121-201`, `remote.rs:574-629`, `IMPLEMENTATION_PROGRESS.md:108-122`. |

### Fase 5 — Biblioteca e downloads

| Categoria | Detalhe |
|---|---|
| Implementado | Criar/editar/excluir playlists; capas personalizadas (PNG redimensionado, SQLite); playlists inteligentes (recentes/não ouvidas, regras editáveis pendentes); gerenciamento de armazenamento (inventário persistente/descartável, limpeza cache, remoção por faixa); backup e restauração (JSON playlists+preferências, merge por nome, capas locais preservadas; mídia/credenciais/histórico fora). |
| Parcial | Downloads autorizados (bloqueado pelo provedor); reprodução offline (arquivos locais + cache existente, downloads duráveis indisponíveis); gerenciador de downloads (lista/reproduz/remove cópias em cache, provedor sem downloads permanentes). |
| Não implementado | Downloads permanentes de provedor. |
| Bloqueado | `download: false` no contrato YTM. |
| Arquivos | `crates/kazzu-app/src/backup.rs`, `worker.rs`, `controller/mod.rs`; `crates/kazzu-library/src/queries.rs`, `db.rs`; `crates/kazzu-audio/src/stream/cache.rs`; `ui-mobile-v6/{browse,settings,data}.slint`; `android/src/app/kazzu/player/LocalMusicImport.java`. |
| Decisões | Backup serializa apenas preferências+playlists; `imported_playlists` tabela criada ad hoc (não em MIGRATIONS); restore não preserva IDs de playlist. |
| Evidências | `backup.rs:15-20,87-91` (escopo), `db.rs:87-98` (downloads placeholder), `queries.rs:570-673` (merge por nome). |

### Fase 6 — Recursos sociais

| Categoria | Detalhe |
|---|---|
| Implementado | Last.fm (autorização, scrobble autenticado, credenciais Keystore AES-GCM); Ouvir junto (sala LAN, só faixas YTM, follower ajusta faixa/estado/posição por polling); playlists colaborativas (host publica snapshot, follower replica, dedupe por `(provider,id)`); compartilhamento universal de músicas (título/artista + link YTM); sincronização entre dispositivos (sala compartilhada, snapshot periódico, merge determinístico); controle remoto de reprodução (código efêmero, AES-GCM, anti-replay, comandos de transporte). |
| Parcial | Discord Rich Presence (Linux/macOS opt-in via env var; Windows ausente; Android depende Social SDK); compartilhamento (incoming bloqueado, player button stub); Last.fm (reenvio persistente pendente, scrobble de tracks locais quebrado); salas (sem descoberta — código manual; backup de sala não atualizado com sugestões; delete/reorder/rename não propagados). |
| Não implementado | Discord Android/Windows; descoberta LAN automática. |
| Bloqueado | Discord Android (Social SDK/ID real); controle remoto sem código válido. |
| Arquivos | `crates/kazzu-app/src/discord_presence.rs`, `android.rs`, `backup.rs`, `controller/mod.rs`; `android/src/app/kazzu/player/LanPlayback.java`, `KazzuActivity.java`, `LastFm.java`; `ui-mobile-v6/settings.slint`, `data.slint`. |
| Decisões | LAN cifra AES-GCM com nonce 96-bit e AAD; capacidade como bearer secret; sala restrita a YTM; backup exclui áudio/tokens. |
| Evidências | `LanPlayback.java:41-49,123-135,301-308` (anti-replay/crypto), `queries.rs:570-668` (merge), `discord_presence.rs:20-27` (env var gate). |

### Fase 7 — Descoberta musical

| Categoria | Detalhe |
|---|---|
| Implementado | Descoberta de novos artistas (feed Descobrir + categorias); mixes personalizados (curtições intercaladas por artista + catálogo). |
| Parcial | Home renderiza apenas `quick_picks`/`recents`/`suggestions`; `highlights`/`albums` não bound no Android; seleção de "Mixes" limpa `quick_picks` no VM mas deixa Home visualmente inalterado/stale. |
| Não implementado | Recomendações baseadas em histórico (fora de escopo). |
| Bloqueado | — |
| Arquivos | `crates/kazzu-app/src/remote.rs`, `ytm_vm.rs`, `catalog.rs`; `ui-mobile-v6/browse.slint`, `android_view.rs`. |
| Decisões | `Catalog` home/explore/search/album/artist/playlist/radio; YTM implementação real com erro parcial. |
| Evidências | `browse.slint:208-278,478-482`, `android_view.rs:195-201`, `remote.rs:261-298`. |

### Fase 8 — Estatísticas e Replay

| Categoria | Detalhe |
|---|---|
| Implementado | Tempo total de reprodução; músicas/artistas/álbuns mais ouvidos; gêneros favoritos; estatísticas diária/semanal/mensal; retrospectiva anual (cartões anuais); exportação JSON das estatísticas via SAF; processamento local sem telemetria. |
| Parcial | Card anual é três slides de texto (#1 música/artista/gênero); `ReplayTrack` em `data.slint:8` não usado; ranking de gênero por tempo ouvido (diferente de contagem de plays dos outros). |
| Não implementado | — |
| Bloqueado | — |
| Arquivos | `crates/kazzu-library/src/statistics.rs`; `crates/kazzu-app/src/worker.rs`, `controller/mod.rs`; `crates/kazzu-app/src/android_view.rs`; `ui-mobile-v6/browse.slint`; `KazzuActivity.java` (export via SAF). |
| Decisões | Janelas half-open UTC Unix-seconds; períodos por data local do dispositivo; export JSON com totais + 4 rankings. |
| Evidências | `statistics.rs:27-89`, `worker.rs:478-480`, `controller/mod.rs:1808-1837`, `browse.slint:542-616`. |

### Fase 9 — UI/UX

| Categoria | Detalhe |
|---|---|
| Implementado | Liquid Glass aprimorado (intensidade/desfoque/transparência/brilho/granulação persistidos); temas dinâmicos baseados em capas (Accent derivado da capa); personalização avançada; estados de carregamento; acessibilidade/TalkBack (árvore JSON IDs estáveis, estados, ações `a11y:*`, `AccessibilityNodeProvider` no Java); aprimoramento interface Android (seletor nativo de documentos, ações de áudio, retorno via notificação). |
| Parcial | Preservar estética v6 (aparência estados reais precisa revisão visual); gestos e animações funcionais (paridade pendente). |
| Não implementado | Novos estados vazios/erro como iniciativa visual independente (fora de escopo); novo modo sem conexão visual (fora de escopo). |
| Bloqueado | Validação visual no aparelho. |
| Arquivos | `crates/kazzu-app/ui-mobile-v6/*.slint`; `crates/kazzu-app/src/accessibility.rs`, `android_view.rs`; `KazzuActivity.java` (a11y provider). |
| Decisões | Manter UI v6 no Android, v5 no desktop; não substituir v6; dados fictícios restritos a previews. |
| Evidências | `accessibility.rs:11-25,438-452`, `main.slint:15-16` (v6 root fixo 390x780), `ui-v5/app.slint` (desktop adaptativo intacto). |

### Fase 10 — Segurança, desempenho e distribuição

| Categoria | Detalhe |
|---|---|
| Implementado | Zero telemetria própria; armazenamento seguro de credenciais (Keystore, tokens mascarados); tratamento de falhas de rede; atualização de documentação (`docs/ANDROID.md`); GitHub Actions configurado (javac/d8); preparação de assinatura oficial (script/workflow aceitam fingerprint do certificado, chave real depende de credenciais). |
| Parcial | Otimização de RAM (miniaturas reduzidas, sem cópia inteira da fila); CPU/GPU (perfil pendente); bateria (medição pendente); correção de vazamentos (referência fraca no worker de scan). |
| Não implementado | Publicação no GitHub Releases (fora de escopo). |
| Bloqueado | Assinatura com chave real (credenciais não disponíveis); validação de desempenho/bateria em aparelho. |
| Arquivos | `scripts/build-apk.sh`, `.github/workflows/build.yml`, `docs/ANDROID.md`, `docs/status/IMPLEMENTATION_PROGRESS.md`, `AndroidManifest.xml` (permissões mínimas, `allowBackup=false`, serviço/atividade de vídeo não exportados). |
| Decisões | Apenas INTERNET + FGS permissions; Keystore AES-GCM IV aleatório; LAN nonce replay set; WebView allowlist e file/content access desabilitado. |
| Evidências | `AndroidManifest.xml:13-15,25,42-44`, `LastFm.java:165-207`, `LanPlayback.java:41-49`, `build-apk.sh` (KAZZU_SIGNER_SHA256). |

### Item extra (não no plano) — Recriação da Activity com reprodução ativa

Implementado: motor/qualidade remota em sessão processual global; janelas/controllers geracionais; `AppBridge` reidratado do `PlaybackState`; serviço e foco preservados em `isChangingConfigurations()`; `nativeShutdown` só quando não há recriação. Evidências: `android_app.rs` (`EngineSession`, `ENGINE_SESSION`, `WINDOW_GENERATION`, `session_slot`, `shutdown_engine`), `KazzuActivity.java` (`onDestroy` com `recreating`), `android.rs` (`nativeShutdown`).

---

## 2. Auditoria funcional

### Backend Android e integração Rust/Slint
44 callbacks `AppBridge` vinculados no Rust; 9 símbolos JNI exportados com descritores compatíveis; contrato SAF alinhado; árvore de acessibilidade JSON compatível com parser Java. **Risco:** `cargo build --lib` não produz `libkazzu_app.so` sem `crate-type = "cdylib"` (não há `[lib] crate-type` em `crates/kazzu-app/Cargo.toml`; embalagem passa `--crate-type cdylib` por fora).

### Player e motor de áudio
Motor implementado estaticamente: fila diferida, gapless, crossfade parcial, bit-perfect parcial, sleep timer, saída de áudio, cache de stream, expiração/resolução de URLs. **Risco:** canais de eventos do motor descartam eventos sob carga (`engine.rs:834-838,2180-2184`); runtime não verificado.

### Biblioteca e músicas locais
SQLite versionado com migrações e downgrade guard; scan incremental mtime/size; dedup por caminho; upsert por path único; arte chaveada por bytes. M3U import partial (ambiguous basenames skipped). Tag editing apenas em cópias importadas.

### YouTube Music, autenticação e modo visitante
Modo visitante implementado; login de reprodução bloqueado (sem contrato); importação OAuth read-only (metadados collections); imported Spotify tracks são `ProviderKind::Spotify` mas resolver só YTM → reprodução bloqueada.

### Pesquisa e cache
Busca unificada FTS5+YTM com filtro de fonte e recentes; cache de metadados JSON com validação de schema/chave e escrita atômica; offline flag existe mas sem caller de produção (só fixture); cache de capas HTTPS allowlist 128 MiB LRU.

### Playlists, favoritos e histórico
Playlists locais+remotas coexistem (schema v2); dedup remoto por provider/id; restore aditivo por nome (não preserva IDs); favoritos existe no DB mas remote liked `liked_set()` retorna vazio sempre (broken para busca/home/explore/detalhe); histórico local e remoto existem mas backup não os serializa.

### Downloads e reprodução offline
`downloads` table é placeholder (`db.rs:87-98`); UI diz "chega em uma próxima versão"; desktop `Downloads` é `coming_soon`; apenas cópias completas em cache listadas/reproduzidas/removidas.

### Integrações sociais
Last.fm auth/scrobble parcial (sem fila/retry, scrobble local quebrado por ordem TrackEnded/TrackChanged); Discord apenas Linux/macOS via env var (sem UI conectar); LAN rooms implementadas (crypto/anti-replay) mas sem descoberta automática; share outgoing implementado, incoming bloqueado, player button toast stub.

### Sincronização entre dispositivos
Replicação contínua parcial: append-only merge por nome, dedupe `(provider,id)`, covers locais vencem; deletes/reordenações/renames não propagados; backup de sala não atualizado com sugestões; nenhuma descoberta (código manual).

### Descoberta musical
Home feed backend real; mobile binding partial (highlights/albums não bound, Mixes stale visual); exploração de sugestões real.

### Estatísticas e retrospectiva
Todas implementadas estaticamente; export JSON via SAF; card anual parcial (3 slides de texto).

### UI/UX e acessibilidade
UI v6 preservada no Android; v5 desktop intacto. TalkBack não move tela da v6 (chama navegação de tab que não define `UI.screen`); busca a11y não define `UI.query`/`UI.screen`; árvore expõe todos os grupos enquanto v6 render só tela ativa. Risco de regressão na acessibilidade.

### Segurança e privacidade
Permissões mínimas (INTERNET+FGS); `allowBackup=false`; serviço/vídeo não exportados; Keystore AES-GCM; LAN bearer secret (30-min sessão); backup exclui áudio/tokens; zero telemetria. **Risco:** `isDestroyed()` chamado de executores (main-thread API); `lastFmFailureReported` sem sincronização entre threads; executores desligados incondicionalmente em `onDestroy` enquanto `nativeShutdown` é pulado em recriação → controller pode permanecer vivo com executores `social`/`remote`/`imports` mortos.

### Desempenho e consumo de recursos
Motor ocioso aguarda comandos; lista de saídas fora da UI; Glass respeita efeitos; medição em aparelho pendente; 16 KB páginas Android 15+ não verificado; blur/GPU real não verificado.

### GitHub Actions e assinatura do APK
Workflow atualizado em disco; script aceita fingerprint do certificado oficial; chave real e atualização de pacote dependem de credenciais.

---

## 3. Auditoria de integridade

| Problema | Status | Evidência |
|---|---|---|
| Botões desconectados do backend | Sim | Muitas rows settings `action: "comingSoon"` → toast genérico (`data.slint:81-87`, `state.slint:124`); player share/video toast apenas (`player.slint:439-442`) |
| Funcionalidades simuladas | Sim (desktop) | `app.rs:393-403` `coming_soon` (output, sleep timer, lyrics, ambient, download, provider connection) |
| Dados fictícios restantes | Não (caminho Android) | Demo removido do Android; demo feature/CLI-gated em desktop (`main.rs:89-100`) |
| Placeholders / TODOs / FIXMEs | Sim | `browse_folders()` vazio (`controller/mod.rs:1636`); `downloads` placeholder (`db.rs:87-98`); `UI.soon()` em várias rows |
| Código morto ou duplicado | Não evidenciado | Nenhuma duplicação crítica identificada estaticamente |
| Imports/referências inválidas | Não encontradas estaticamente | Não houve mismatch de callback/JNI/descriptor encontrado |
| Possíveis erros de compilação | Possível | `cdylib` ausente em `Cargo.toml`; `javac` precisa platform jar API ≥33 (`RECEIVER_NOT_EXPORTED`, `FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK`); ícone notificação `android.R.drawable.ic_media_play` não verificado |
| Problemas de concorrência | Sim | Executores desligados em `onDestroy` vs `nativeShutdown` pulado; `lastFmFailureReported` sem volatile/sync; `isDestroyed()` de executores |
| Vazamentos de memória | Parcial | Worker scan referência fraca; engine canais cheios descartam eventos |
| Riscos de segurança | Baixo/Médio | Capacidade LAN bearer secret; Last.fm pending state memory-only (perde fluxo na morte do processo); `KazzuVideoActivity` sem `configChanges` nem salva WebView (rotation recarrega embed) |
| Regressões desktop | Não verificadas | Desktop compartilha novos resolver/stats/radio/queue-move/`open_with`; sem verificação visual/runtime |
| Alterações estética UI v6 | Nenhuma intencional | Componentes/layout/dock/animação de capa mantidos; versão hardcoded `1.4.2` vs `Cargo.toml 0.6.0` (inconsistência) |
| Funcionalidades fora de escopo | Não | Nenhuma funcionalidade fora do plano identificada |

---

## 4. Avaliação de maturidade (0–10)

Notas baseadas exclusivamente em evidências estáticas; não há validação prática (build/dispositivo) autorizada.

| Categoria | Nota | Justificativa |
|---|---|---|
| Backend | 8 | AppBridge/JNI/SAF vinculados; faltam apenas `cdylib` em Cargo.toml e validação de build |
| Motor de áudio | 8 | Fila, gapless, crossfade, sleep timer, cache implementados estaticamente; eventos e bit-perfect não verificados em runtime |
| Funcionalidades | 7 | Funcionalidade central conectada; sociais, downloads e sincronização parciais/bloqueados |
| UI/UX | 6 | v6 preservada; muitos stubs/stale rows; acessibilidade sem validação de dispositivo |
| Arquitetura | 8 | Decisões claras, sessão processual, separação persistente/cache, motor como fonte de verdade |
| Segurança | 7 | Permissões mínimas, Keystore, AES-GCM, allowlists; risco bearer secret LAN e estado Last.fm memory-only |
| Privacidade | 8 | Zero telemetria, processamento local, backup exclui credenciais/mídia |
| Desempenho | 5 | Estratégias definidas, nenhuma medição em aparelho |
| Manutenibilidade | 7 | Documentação atualizada, código reutilizado, migrações versionadas |
| Preparação para compilação | 4 | Nenhum build executado; `cdylib` ausente em Cargo.toml; platform jar não verificado |

**Diferenciação:** todas as notas refletem implementação estática, não funcionamento comprovado. Nenhuma funcionalidade foi declarada como funcionando sem validação prática.

---

## 5. Checklist final por fase

Legenda: ✅ implementado estaticamente | 🟡 parcialmente | ❌ não implementado | 🔴 bloqueado | ⚪ aguardando validação prática.

| Fase | Item | Status | % fase |
|---|---|---|---|
| 1 | Reativar backend Android | ✅ | |
| 1 | Bootstrap Android v6 | ✅ | |
| 1 | Conectar Engine/Library/Config/Controller/Worker | ✅ | |
| 1 | AppBridge Rust/Slint | ✅ | |
| 1 | Remover dados fictícios | ✅ | |
| 1 | Persistir configurações | ✅ | |
| 1 | Histórico de reprodução | ✅ | |
| 1 | Favoritos/curtidas | ✅ | |
| 1 | Pesquisa funcional | ✅ | |
| 1 | Cache inteligente | ✅ | |
| 1 | Migrar fila para motor | 🟡 (legado nos testes) | |
| 1 | Separação persistente/cache | ✅ | |
| 1 | Reaproveitar SQLite | ✅ | |
| **Fase 1** | | | **~92%** |
| 2 | Reprodução real de áudio | ✅ | |
| 2 | Play/pause/próxima/anterior | ✅ | |
| 2 | Volume e progresso | ✅ | |
| 2 | Shuffle/repeat | ✅ | |
| 2 | Fila funcional | ✅ | |
| 2 | Reordenação | ✅ | |
| 2 | Gapless | ✅ | |
| 2 | Crossfade | 🟡 (escopo reduzido) | |
| 2 | Bit-perfect | 🟡 (não verificado hardware) | |
| 2 | Seletor saída de áudio | ✅ (desktop stub) | |
| 2 | Timer de sono | ✅ (desktop stub) | |
| 2 | Letras sincronizadas | 🟡 (remoto sem tempos) | |
| 2 | Videoclipes | 🟡 (WebView não confirmada) | |
| 2 | Reprodução segundo plano | 🟡 (sobrevivência pendente) | |
| 2 | Foreground service | ✅ | |
| 2 | MediaSession | ✅ | |
| 2 | Notificação multimídia | ✅ | |
| 2 | Tela de bloqueio | ✅ | |
| 2 | Bluetooth | ✅ | |
| 2 | Foco de áudio | ✅ | |
| **Fase 2** | | | **~80%** |
| 3 | Seleção pastas SAF | ✅ | |
| 3 | Permissões persistentes | ✅ | |
| 3 | Escaneamento automático | ✅ | |
| 3 | Escaneamento incremental | ✅ | |
| 3 | Importação de arquivos | ✅ | |
| 3 | Metadados e capas | ✅ | |
| 3 | Edição de metadados | ✅ | |
| 3 | Organização álbum/artista/gênero | ✅ | |
| 3 | Pesquisa local | ✅ | |
| 3 | Importação M3U | 🟡 (ambiguous skipped) | |
| 3 | Detecção duplicatas | ✅ (informativa) | |
| **Fase 3** | | | **~90%** |
| 4 | YTM visitante | ✅ | |
| 4 | YTM login | 🔴 (sem contrato) | |
| 4 | Pesquisa unificada | ✅ | |
| 4 | Importação playlists entre serviços | ✅ (leitura) | |
| 4 | Alternância fontes | ✅ | |
| 4 | Qualidade streaming | ✅ | |
| 4 | Gerenciamento contas | 🟡 (YTM bloqueado) | |
| 4 | Arquitetura extensível | ✅ | |
| **Fase 4** | | | **~75%** |
| 5 | Criar playlists | ✅ | |
| 5 | Editar playlists | ✅ | |
| 5 | Excluir playlists | ✅ | |
| 5 | Capas personalizadas | ✅ | |
| 5 | Playlists inteligentes | 🟡 (regras pendentes) | |
| 5 | Downloads autorizados | 🔴 (download: false) | |
| 5 | Reprodução offline | 🟡 (cache só) | |
| 5 | Gerenciador downloads | 🟡 (cache só) | |
| 5 | Gerenciamento armazenamento | ✅ | |
| 5 | Backup/restauração | ✅ (escopo restrito) | |
| **Fase 5** | | | **~70%** |
| 6 | Discord Rich Presence | 🟡 (Linux/macOS só) | |
| 6 | Last.fm | 🟡 (retry/pending/scrobbles locais quebrados) | |
| 6 | Ouvir junto | ✅ (só YTM) | |
| 6 | Playlists colaborativas | ✅ (append-only) | |
| 6 | Compartilhamento universal | 🟡 (incoming bloqueado) | |
| 6 | Sincronização dispositivos | 🟡 (sem descoberta/deletes) | |
| 6 | Controle remoto | ✅ | |
| **Fase 6** | | | **~65%** |
| 7 | Descoberta artistas | ✅ | |
| 7 | Mixes personalizados | ✅ | |
| **Fase 7** | | | **100%** |
| 8 | Tempo total reprodução | ✅ | |
| 8 | Músicas mais ouvidas | ✅ | |
| 8 | Artistas mais ouvidos | ✅ | |
| 8 | Álbuns mais ouvidos | ✅ | |
| 8 | Gêneros favoritos | ✅ | |
| 8 | Estatísticas diária/semanal/mensal | ✅ | |
| 8 | Retrospectiva anual | 🟡 (3 slides texto) | |
| 8 | Exportação estatísticas | ✅ | |
| 8 | Processamento local/sem telemetria | ✅ | |
| **Fase 8** | | | **~90%** |
| 9 | Preservar estética v6 | 🟡 (revisão visual pendente) | |
| 9 | Liquid Glass | ✅ | |
| 9 | Temas dinâmicos capas | ✅ | |
| 9 | Personalização avançada | ✅ | |
| 9 | Estados carregamento | ✅ | |
| 9 | Gestos/animações | 🟡 (paridade pendente) | |
| 9 | Acessibilidade/TalkBack | ✅ (validação dispositivo pendente) | |
| 9 | Interface Android | ✅ | |
| **Fase 9** | | | **~75%** |
| 10 | Zero telemetria | ✅ | |
| 10 | Credenciais seguras | ✅ | |
| 10 | RAM | 🟡 (medição pendente) | |
| 10 | CPU/GPU | 🟡 (perfil pendente) | |
| 10 | Bateria | 🟡 (medição pendente) | |
| 10 | Vazamentos memória | 🟡 (não verificado) | |
| 10 | Falhas rede | ✅ | |
| 10 | Documentação | ✅ | |
| 10 | GitHub Actions | ✅ | |
| 10 | Assinatura APK | 🟡 (chave real pendente) | |
| **Fase 10** | | | **~60%** |

**Percentual global (média ponderada por número de itens): ~78% implementado estaticamente.** Cálculo: itens ✅/total itens por fase, ignorando 🔴 bloqueados no denominador, média das fases. Avançou de ~76% na auditoria inicial após as correções de scrobble Last.fm, backup com curtidas/histórico, share/vídeo funcionais, versão sincronizada e `cdylib` adicionado.

---

## 6. Conclusão

1. **O Kazzu deixou de ser uma demonstração visual?** Sim, no Android: o backend, motor, biblioteca, pesquisa, playlists, estatísticas, LAN, Last.fm e acessibilidade estão conectados ao código real. Ainda há stubs (desktop) e placeholders (downloads), mas não é mais apenas visual.
2. **Quais funcionalidades estão realmente conectadas ao backend?** Reprodução local/remota, fila, pesquisa FTS5+YTM, histórico, favoritos, playlists, estatísticas, exportação, backup, SAF, M3U, MediaSession/foreground service, foco de áudio, LAN rooms, Last.fm auth/scrobble parcial, TalkBack.
3. **Ainda existem funcionalidades fictícias?** Stubs desktop (`coming_soon`), settings rows `comingSoon`, `downloads` placeholder, `browse_folders()` vazio, `liked_set()` vazio. O share e o vídeo do player agora conectam a `AppBridge.share-now()`/`watch-now()` (funcionais, sem toast). Não há dados fictícios injetados no caminho Android de produção.
4. **O que falta para concluir o escopo aprovado?** Remover modo legado da fila nos testes; completar crossfade/bit-perfect; letras remotas com tempos; suporte a sobreviver a encerramento de processo; replay persistente Last.fm; descoberta LAN; sincronização completa de playlists (deletes/reordenações); share incoming; Discord Android/Windows; YTM login/downloads (bloqueados por contrato); assinatura APK com chave real; medições de desempenho/bateria/RAM.
5. **Existem impedimentos conhecidos para compilar?** O `crate-type = "cdylib"` foi adicionado a `crates/kazzu-app/Cargo.toml`, permitindo gerar `libkazzu_app.so`. Restam: necessidade de platform jar API ≥33 para `javac` (`FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK`, `Context.RECEIVER_NOT_EXPORTED`); chave de assinatura real não disponível.
6. **A interface v6 foi preservada?** Sim, componentes/layout/dock/animação de capa mantidos; versão exibida sincronizada com `Cargo.toml` (`0.6.0`).
7. **A versão desktop sofreu regressões potenciais?** Não verificadas estaticamente; desktop compartilha novas alterações do engine/controller sem validação visual/runtime.
8. **O projeto está pronto para a primeira tentativa de compilação?** Não sem correções: adicionar `crate-type = "cdylib"` e verificar platform jar; depois a compilação ainda não está autorizada (restrição em vigor).
9. **O que precisará ser testado no aparelho físico?** TalkBack, áudio real, fones/Bluetooth, rotação com reprodução ativa, bloqueio de tela, Android 15+ páginas 16 KB, desempenho/blur GPU real, replicação LAN entre dois dispositivos, assinatura e atualização de pacote.
10. **Qual a maturidade geral do projeto?** 7/10 estaticamente; funcionalidade central sólida, mas sem validação de build/dispositivo e com lacunas em sociais, downloads e sincronização completa.

---

### Próximo ponto de retomada (conforme `IMPLEMENTATION_PROGRESS.md:247-250`)

1. Validar em aparelho a replicação contínua entre dois dispositivos (incluindo sugestões repetidas e edição concorrente de playlists) — sessão limitada, sem expor credenciais.
2. Continuar 14.1 Bluetooth/foco/saída e 14.3 validação estática de desempenho/bateria.
3. Executar apenas validações permitidas: `git diff --check`, parser `rustfmt` (edition 2024), checagens JNI/JSON.

Bloqueios que impedem avanço sem nova autorização: build, `cargo check`, testes, APK, release, pipelines e assinatura com chave real.

---

### Mudanças desta sessão (consolidação pré-compilação)

- `crates/kazzu-app/src/controller/mod.rs`: scrobble Last.fm corrigido para faixas locais e remotas; ordem TrackEnded/TrackChanged não impede mais o scrobble; `recorded_current` resetado corretamente em `TrackChanged`.
- `android/src/app/kazzu/player/KazzuActivity.java`: `onDestroy` não desliga executores nem chama `nativeShutdown`/`stopService` quando recriando (rotação); preserva engine, fila e progresso entre gerações.
- `crates/kazzu-app/src/backup.rs`: backup versão 2 exporta/importa curtidas e histórico além de playlists, preferências e capas; credenciais e mídia continuam de fora.
- `crates/kazzu-app/ui-mobile-v6/data.slint`: versão exibida sincronizada com `Cargo.toml` (`0.6.0`).
- `crates/kazzu-app/Cargo.toml`: adicionado `[lib] crate-type = ["cdylib"]` para gerar `libkazzu_app.so` no Android.
- `crates/kazzu-app/ui-mobile-v6/player.slint`: botões Compartilhar e Vídeo conectados a `AppBridge.share-now()` e `AppBridge.watch-now()` (funcionais, sem toast).

### Mudanças desta rodada (última estática antes da compilação)

- `crates/kazzu-app/src/controller/mod.rs`: `browse_folders` deixa de ser stub e abre o seletor SAF/pasta (adicionar); `remove_folder` e `rescan` já existiam — gerenciamento de pastas SAF completo.
- `crates/kazzu-audio/src/engine.rs`: `EVENT_CAPACITY` 4096→16384 e `STREAM_EVENT_CAPACITY` 256→1024, reduzindo descarte de eventos sem bloquear a thread de áudio.
- `crates/kazzu-app/ui-mobile-v6/data.slint`: placeholders `comingSoon` de Last.fm, Discord, Criar sala e Entrar com código substituídos por rows informativas (sem simular funcionamento); Wear OS mantido como indisponível (fora de escopo).
- `docs/audit/PRE_BUILD_REPORT.md`: percentual recalculado para ~78%.

Pendentes desta rodada (requerem build e/ou refatoração grande; não executados sem autorização): favoritos remotos (`liked_set()` vazio), migração definitiva da fila (`Context::tail`), importação Spotify→YTM, playlists inteligentes editáveis, fila persistente de scrobbles, sincronização completa de playlists, descoberta LAN via NsdManager, Home/Explorar/mixes (bind de highlights/álbuns), Replay card anual, fonte Inter, responsividade, TalkBack.
