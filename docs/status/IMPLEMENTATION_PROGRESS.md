# Kazzu — Progresso de implementação

Referência: [escopo definitivo](IMPLEMENTATION_PLAN.md). Status se refere apenas a este trabalho; funções já existentes ainda requerem integração e verificação. Nenhum build, cargo check, teste, APK ou release foi executado.

## Impedimentos e riscos

- Toda implementação estática ainda precisa de validação no aparelho e de verificação de compatibilidade desktop. Esta rodada não autoriza execução de compilação, testes ou pipeline.
- Os bootstraps Android e desktop entregam ao motor a fila completa, inclusive entradas remotas diferidas. O caminho antigo de Context::tail permanece disponível para os chamadores de teste que habilitam o catálogo sem resolver no motor.
- O catálogo YouTube Music existente funciona como visitante e não anuncia download autorizado. A importação OAuth consulta a API de coleções suportada. Login de reprodução e download permanente não têm contrato de provedor implementado.
- Para o Android, a ponte do sistema é Java mínimo compilado por javac/d8 no empacotamento existente. O motor, o controller e a UI continuam em Rust/Slint.

## Fase 1 — Backend Android

- [x] **Implementado no código, aguardando validação** — Reativar o backend Android. Bootstrap NativeActivity inicializa o backend compartilhado.

- [x] **Implementado no código, aguardando validação** — Criar bootstrap Android específico para a UI v6. Montagem em android_app.rs com AndroidView e ciclo de vida.

- [x] **Implementado no código, aguardando validação** — Conectar Engine, Library, Config, Controller e Worker. Composição via AndroidBackend, ThreadedRunner e canais do motor.

- [x] **Implementado no código, aguardando validação** — Criar AppBridge para conectar Rust e Slint. AppBridge exporta modelos e comandos usados pela UI v6.

- [x] **Implementado no código, aguardando validação** — Remover dados fictícios dos caminhos de produção. Removidos Demo, capas fictícias referenciadas, relógio de reprodução simulado e números fixos de Replay do caminho Android.

- [x] **Implementado no código, aguardando validação** — TOML interno para volume, preferências do catálogo, Glass, tema, movimento, formas e opções do player.

- [x] **Implementado no código, aguardando validação** — Histórico local e remoto guardado no SQLite e mostrado na v6. A integração mede avanços de áudio e descarta saltos de posição incompatíveis com o relógio da amostra.

- [x] **Implementado no código, aguardando validação** — Favoritos e curtidas. Alternância na faixa atual e coleção Curtidos usam Controller e armazenamento existente.

- [x] **Implementado no código, aguardando validação** — Pesquisa funcional. Pesquisa FTS5 e YTM via worker com filtro de fonte e buscas recentes.

- [x] **Implementado no código, aguardando validação** — Cache inteligente. Capas, metadados YTM e cache LRU de streaming recebem o diretório descartável.

- [x] **Implementado em produção, aguardando validação** — Os dois bootstraps usam a fila do motor com resolução diferida. O controller guarda estado do rádio, sem cauda em produção. Falta retirar o modo legado usado pelos testes.

- [x] **Implementado no código, aguardando validação** — Bootstrap usa getFilesDir para TOML/SQLite/estado YTM e getCacheDir para capas, metadados e streams.

- [x] **Implementado no código, aguardando validação** — Reaproveitar o SQLite existente. Worker abre a biblioteca SQLite e o cache em caminhos separados.

## Fase 2 — Player

- [x] **Implementado no código, aguardando validação** — Reprodução real de áudio. Player encaminha faixas locais e remotas ao motor existente.

- [x] **Implementado no código, aguardando validação** — Play/pause, próxima e anterior. Controles v6 e MediaSession encaminham comandos reais.

- [x] **Implementado no código, aguardando validação** — Volume e progresso. Volume, posição, disponibilidade de seek e eventos conectados.

- [x] **Implementado no código, aguardando validação** — Shuffle e repeat. Controles e projeção baseados no motor Android.

- [x] **Implementado no código, aguardando validação** — Fila funcional. Adicionar, avançar, selecionar e remover por ID de entrada no Android.

- [x] **Implementado no código, aguardando validação** — Reordenação da fila. Movimentação de itens pendentes ou resolvidos por ID estável no Android.

- [x] **Implementado no código, aguardando validação** — Gapless. Motor existente mantém prefetch do próximo stream, inclusive resolução remota tardia.

- [x] **Implementado no código, aguardando validação** — Mistura linear opcional para pares locais WAV/FLAC de taxa e canais compatíveis. O caminho gapless cobre os outros casos.

- [x] **Implementado no código, aguardando validação** — Bit-perfect em dispositivos compatíveis. Caminho existente e indicação restrita ao motor. A rota física depende do sistema.

- [x] **Implementado no código, aguardando validação** — Seletor de saída de áudio. Enumeração CPAL fora da thread Slint e alteração do sink no motor.

- [x] **Implementado no código, aguardando validação** — Timer de sono. Timer no motor pausa a reprodução no prazo configurado pela v6.

- [ ] **Parcial** — Letras sincronizadas. Leitura de arquivos LRC locais e sincronização em milissegundos. Letras remotas são exibidas com atribuição e sem inferir tempos.

- [x] **Implementado no código, aguardando validação** — WebView com o player oficial para vídeos que permitem incorporação, com opção de abrir no YouTube.

- [ ] **Parcial** — Reprodução em segundo plano. Serviço Android mantém os controles ao minimizar ou bloquear o aparelho. Sobrevivência ao encerramento da Activity/processo continua pendente.

- [x] **Implementado no código, aguardando validação** — Foreground service. Serviço não exportado mediaPlayback e permissões no manifesto.

- [x] **Implementado no código, aguardando validação** — MediaSession. Metadados, estado, seek e callbacks nativos ligados ao controller.

- [x] **Implementado no código, aguardando validação** — Notificação multimídia. MediaStyle com ações e retorno à Activity.

- [x] **Implementado no código, aguardando validação** — Controle pela tela de bloqueio. MediaSession publica ações e metadados.

- [x] **Implementado no código, aguardando validação** — Controle por Bluetooth. MediaSession processa transporte; perda da rota pede pausa.

- [x] **Implementado no código, aguardando validação** — Gerenciamento de foco de áudio. AudioFocusRequest e pausa ao perder foco.

## Fase 3 — Músicas locais

- [x] **Implementado no código, aguardando validação** — Seleção de pastas via SAF. Seletor ACTION_OPEN_DOCUMENT_TREE ligado ao menu Música local.

- [x] **Implementado no código, aguardando validação** — Permissões persistentes. Concessão de leitura persistente e reabertura da lista de árvores.

- [x] **Implementado no código, aguardando validação** — Escaneamento automático. Espelho SAF reavaliado ao voltar à Activity, com scan do índice existente.

- [x] **Implementado no código, aguardando validação** — Escaneamento incremental. Compara tamanho e data de modificação quando o provedor informa os dois.

- [x] **Implementado no código, aguardando validação** — Importação de arquivos. Cópia atômica em arquivos privados acessíveis ao decodificador.

- [x] **Implementado no código, aguardando validação** — Leitura de metadados e capas. Scanner SQLite e bibliotecas de leitura existentes aplicados às cópias importadas.

- [x] **Implementado no código, aguardando validação** — Edição de metadados. Edição de tags nas cópias privadas, com gravação atômica e reindexação.

- [x] **Implementado no código, aguardando validação** — Biblioteca navega por álbuns, artistas e gêneros reais do SQLite.

- [x] **Implementado no código, aguardando validação** — Pesquisa local. Filtro Biblioteca na busca usa FTS5 existente.

- [x] **Implementado no código, aguardando validação** — Importação de playlists M3U. Seletor de M3U/M3U8 e importação transacional. Caminhos ambíguos ou sem faixa indexada são ignorados e reportados.

- [x] **Implementado no código, aguardando validação** — Detecção de duplicatas. Busca sob demanda por tamanho e SHA-256 das cópias importadas.

## Fase 4 — Provedores

- [x] **Implementado no código, aguardando validação** — YouTube Music no modo visitante. Catálogo configurado com diretórios separados e resolução sob demanda.

- [ ] **Bloqueado pelo provedor** — Catálogo de reprodução disponível só como visitante. OAuth está ligado à importação oficial de coleções.

- [x] **Implementado no código, aguardando validação** — Pesquisa unificada. Canal existente mergeia faixas FTS5 e YTM.

- [x] **Implementado no código, aguardando validação** — Importação de playlists entre serviços compatíveis. UI de importação reutiliza API OAuth de metadados de YouTube/Spotify sem habilitar Spotify para reprodução.

- [x] **Implementado no código, aguardando validação** — Alternância entre fontes de áudio. Origem local/remota selecionável na busca e suportada em filas mistas.

- [x] **Implementado no código, aguardando validação** — Seleção da qualidade de streaming. Preferência existente persistida e consultada na resolução das faixas remotas.

- [ ] **Parcial** — Last.fm permite autorizar, acompanhar estado e desconectar. Importação de coleções usa OAuth temporário. Conta autenticada de reprodução YTM continua bloqueada pelo contrato disponível.

- [x] **Implementado no código, aguardando validação** — Arquitetura extensível de provedores. Android reutiliza Providers e a interface Catalog.

## Fase 5 — Biblioteca e downloads

- [x] **Implementado no código, aguardando validação** — Criar playlists. Formulário v6 usa worker SQLite.

- [x] **Implementado no código, aguardando validação** — Editar playlists. Renomear, adicionar faixa, mover e remover itens pelo worker existente.

- [x] **Implementado no código, aguardando validação** — Excluir playlists. Ação de confirmação no painel da v6.

- [x] **Implementado no código, aguardando validação** — Capas personalizadas. Seletor de imagem, PNG redimensionado e armazenamento persistente no SQLite.

- [x] **Implementado no código, aguardando validação** — Playlists inteligentes. Coleções dinâmicas de faixas adicionadas recentemente e ainda não ouvidas. Regras editáveis são uma extensão pendente.

- [ ] **Bloqueado pelo provedor** — O contrato do YouTube Music anuncia download: false. Cache temporário permanece disponível.

- [ ] **Parcial** — Reprodução offline. Arquivos locais e streams completos no cache existente. Downloads duráveis de provedor ainda indisponíveis.

- [ ] **Parcial** — Lista, reprodução offline e remoção de cópias completas em cache. O provedor de reprodução não oferece downloads permanentes autorizados.

- [x] **Implementado no código, aguardando validação** — Inventário dos diretórios persistente e descartável, limpeza global do cache de áudio e remoção por faixa.

- [x] **Implementado no código, aguardando validação** — Backup e restauração. Exportação JSON de playlists, preferências, curtidas e histórico; restauração mescla por nome sem duplicar faixas e preserva capas locais. Mídia e credenciais ficam fora do backup.

## Fase 6 — Recursos sociais

- [ ] **Parcial** — Presença opt-in em Linux/macOS via IPC e ID público fornecido pelo usuário. Windows ainda sem integração. Android depende do Social SDK e de ID de aplicativo real.

- [x] **Implementado no código, aguardando validação** — Last.fm. Autorização explícita e scrobble autenticado, com credenciais seladas pelo Android Keystore. Scrobble corrigido para faixas locais e remotas (ordem TrackEnded/TrackChanged). Reenvio persistente pendente.

- [x] **Implementado no código, aguardando validação** — Ouvir junto. Sala LAN autorizada, restrita a faixas do YouTube Music. Seguidor ajusta faixa, estado e posição com consultas periódicas.

- [x] **Implementado no código, aguardando validação** — Playlists colaborativas. Host abre uma playlist para sugestões LAN de faixas YTM; seguidores recebem replicação periódica das playlists do anfitrião. Sugestões repetidas são ignoradas pela chave estável `(provider, id)`.

- [x] **Implementado no código, aguardando validação** — Compartilhamento universal de músicas. Compartilhamento Android de título/artista e link YTM quando disponível.

- [x] **Implementado no código, aguardando validação** — Sala sincroniza reprodução e transfere playlists/capas em operação LAN autorizada. Com a sala compartilhada, o host publica um snapshot periodicamente consultado pelo seguidor; preferências locais não são sobrescritas e a mesclagem é determinística por nome de playlist e identidade da faixa.

- [x] **Implementado no código, aguardando validação** — Controle remoto da reprodução. Sessão LAN explícita, código efêmero, cifragem autenticada, rejeição de replays e comandos de transporte.

Priorizar LAN, P2P e soluções descentralizadas quando tecnicamente viáveis.

## Fase 7 — Descoberta musical

- [x] **Implementado no código, aguardando validação** — Descoberta de novos artistas. Feed Descobrir e categorias reais no Android.

- [x] **Implementado no código, aguardando validação** — Mix próprio de curtidas intercaladas por artista e mixes retornados pelo catálogo. Usa as curtidas existentes.

## Fase 8 — Estatísticas e Replay

- [x] **Implementado no código, aguardando validação** — Tempo total de reprodução. Agregação SQLite de tempo de escuta do histórico local e remoto no aparelho.

- [x] **Implementado no código, aguardando validação** — Músicas mais ouvidas. Ranking no aparelho por reproduções.

- [x] **Implementado no código, aguardando validação** — Artistas mais ouvidos. Ranking no aparelho pelo artista principal cadastrado.

- [x] **Implementado no código, aguardando validação** — Álbuns mais ouvidos. Ranking no aparelho de álbuns.

- [x] **Implementado no código, aguardando validação** — Gêneros favoritos. Ranking no aparelho por tempo de escuta.

- [x] **Implementado no código, aguardando validação** — Estatísticas diárias, semanais e mensais. Janelas de calendário do fuso do Android.

- [x] **Implementado no código, aguardando validação** — Cartões anuais navegáveis com a faixa, o artista e o gênero em destaque, além de totais e rankings da v6.

- [x] **Implementado no código, aguardando validação** — Exportação das estatísticas. JSON exportado via seletor de arquivo do Android.

- [x] **Implementado no código, aguardando validação** — Processamento local, sem telemetria. Consultas SQLite e exportação no aparelho.

## Fase 9 — UI/UX

- [ ] **Parcial** — Preservar a estética atual da v6. Componentes, layout principal, dock e animação de capa foram mantidos. A aparência dos estados reais precisa de revisão visual.

- [x] **Implementado no código, aguardando validação** — Aprimorar o Liquid Glass. Controles de vidro persistidos com intensidade, desfoque, transparência, brilho e granulação.

- [x] **Implementado no código, aguardando validação** — Temas dinâmicos baseados nas capas. Cor de destaque reaproveita Accent derivado da capa.

- [x] **Implementado no código, aguardando validação** — Personalização avançada. Tema, vidro, formas, modo de efeitos e visual do player usam preferências persistentes.

- [x] **Implementado no código, aguardando validação** — Estados de carregamento. Busca, home, detalhe e biblioteca leem estado do backend.

- [ ] **Parcial** — Gestos e animações funcionais. Navegação e gestos visuais usam ações reais quando ligadas ao backend. Validação de paridade pendente.

- [x] **Implementado no código, aguardando validação** — Acessibilidade e TalkBack. A v6 publica uma árvore com IDs estáveis, estados e ações de navegação, fila e controles; `KazzuActivity` expõe a árvore por `AccessibilityNodeProvider` e devolve ações ao `AppBridge`. A validação no aparelho continua pendente.

- [x] **Implementado no código, aguardando validação** — Aprimoramento da interface Android. Seletor nativo de documentos, ações de áudio e retorno à Activity via notificação.

## Fase 10 — Segurança, desempenho e distribuição

- [x] **Implementado no código, aguardando validação** — Zero telemetria própria. Ponte Android e estatísticas operam localmente. Provedores são acessados por funções explícitas. Scrobbles Last.fm exigem opt-in.

- [x] **Implementado no código, aguardando validação** — Sessão Last.fm cifrada com Android Keystore, token de importação OAuth temporário e campos mascarados. A conta de reprodução YTM permanece indisponível.

- [ ] **Parcial** — Tamanho e contagem de miniaturas reduzidos no Android, sem cópia inteira da fila no Controller. Medição no aparelho pendente.

- [ ] **Parcial** — Otimização de CPU e GPU. Motor ocioso aguarda comandos, lista de saídas roda fora da UI e Glass respeita configurações de efeitos. Perfil de CPU/GPU pendente.

- [ ] **Parcial** — Otimização de bateria. Motor ocioso evita polling contínuo. Medir consumo, CPU de fundo e política de pré-carregamento em aparelho.

- [ ] **Parcial** — Correção de vazamentos de memória. Worker de biblioteca usa referência fraca para reagendar scan e pode encerrar quando o app é descartado.

- [x] **Implementado no código, aguardando validação** — Tratamento de falhas de rede. Bridge conecta erros do catálogo/engine, com fallback local e cache quando disponível.

- [x] **Implementado no código, aguardando validação** — Recriação da Activity com reprodução ativa. O motor e a qualidade remota vivem numa sessão processual, cada geração de janela recebe um clone do handle e o `AppBridge` é reidratado pelo snapshot do motor; o serviço e o foco são preservados em mudanças de configuração e o encerramento real chama `nativeShutdown`.

- [x] **Implementado no código, aguardando validação** — Atualização da documentação. docs/ANDROID.md descreve a composição, mídia, SAF, armazenamento e empacotamento atuais.

- [x] **Implementado no código, aguardando validação** — Configuração do GitHub Actions. Caminho Android usa o JDK existente para javac e d8 no script. Workflow atualizado em disco.

- [x] **Preparação implementada, assinatura aguardando validação** — Script e workflow aceitam a impressão digital do certificado oficial. Usar a chave real e conferir atualização do pacote depende de credenciais e execução posteriormente autorizada.


## Validação estática

- Revisados diffs, sintaxe Rust pelo parser do rustfmt (edition 2024), manifesto XML, sintaxe do script Bash, contrato JSON da árvore de acessibilidade e correspondência de nomes e assinaturas entre AppBridge, JNI Rust e ponte Java, incluindo `nativeShutdown` e `nativeSyncPeerPlaylists`.
- Continuam sem verificação a compilação, o áudio no aparelho, a recriação da Activity em rotação, a paridade visual, os providers reais, a navegação do TalkBack, desempenho, assinatura e regressões no desktop.

## Arquivos principais

| Área | Arquivos |
|---|---|
| Android e UI v6 | `android/AndroidManifest.xml`, `android/src/app/kazzu/player/`, `crates/kazzu-app/src/android*.rs`, `crates/kazzu-app/ui-mobile-v6/` |
| Motor e fila | `crates/kazzu-core/src/{playback,provider}.rs`, `crates/kazzu-audio/src/{engine,queue,decoder}.rs`, `crates/kazzu-audio/src/stream/`, `crates/kazzu-app/src/controller/` |
| Aplicativo e dados | `crates/kazzu-app/src/{app,config,catalog,worker,view,vm,platform,event_bridge,backup,lyrics,m3u,playlist_cover,discord_presence}.rs`, `crates/kazzu-library/src/` |
| Empacotamento e documentação | `scripts/build-apk.sh`, `.github/workflows/build.yml`, `docs/ANDROID.md`, `docs/status/IMPLEMENTATION_PROGRESS.md` |

## Mudanças desta sessão (consolidação pré-compilação)

- `crates/kazzu-app/src/controller/mod.rs`: scrobble Last.fm corrigido para faixas locais e remotas; ordem TrackEnded/TrackChanged não impede mais o scrobble; `recorded_current` resetado corretamente em `TrackChanged`.
- `android/src/app/kazzu/player/KazzuActivity.java`: `onDestroy` não desliga executores nem chama `nativeShutdown`/`stopService` quando recriando (rotação); preserva engine, fila e progresso entre gerações.
- `crates/kazzu-app/src/backup.rs`: backup versão 2 exporta/importa curtidas e histórico além de playlists, preferências e capas; credenciais e mídia continuam de fora.
- `crates/kazzu-app/ui-mobile-v6/data.slint`: versão exibida sincronizada com `Cargo.toml` (`0.6.0`).
- `crates/kazzu-app/Cargo.toml`: adicionado `[lib] crate-type = ["cdylib"]` para gerar `libkazzu_app.so` no Android.
- `crates/kazzu-app/ui-mobile-v6/player.slint`: botões Compartilhar e Vídeo conectados a `AppBridge.share-now()` e `AppBridge.watch-now()` (funcionais, sem toast).
- `docs/audit/PRE_BUILD_REPORT.md`: relatório de auditoria pré-compilação criado.

## Mudanças desta rodada (última estática antes da compilação)

- `crates/kazzu-app/src/controller/mod.rs`: `browse_folders` deixa de ser stub e abre o seletor SAF/pasta (adicionar); `remove_folder` e `rescan` já existiam — gerenciamento de pastas SAF completo.
- `crates/kazzu-audio/src/engine.rs`: `EVENT_CAPACITY` 4096→16384 e `STREAM_EVENT_CAPACITY` 256→1024, reduzindo descarte de eventos sem bloquear a thread de áudio.
- `crates/kazzu-app/ui-mobile-v6/data.slint`: placeholders `comingSoon` de Last.fm, Discord, Criar sala e Entrar com código substituídos por rows informativas (sem simular funcionamento); Wear OS mantido como indisponível (fora de escopo).
- `docs/audit/PRE_BUILD_REPORT.md`: percentual recalculado para ~78%.

## Pendentes desta rodada (requerem build e/ou refatoração grande; não executados sem autorização)

- Favoritos remotos: `liked_set()` ainda vazio; requer passar estado `liked` do controller para o `RemoteWorker` (está em `Arc`).
- Migração definitiva da fila: `Context::tail` ainda consumido por `EngineRadio`; refatoração estrutural.
- Importação Spotify→YTM por correspondência; playlists inteligentes editáveis; fila persistente de scrobbles; sincronização completa de playlists; descoberta LAN via NsdManager.
- Home/Explorar/mixes (bind de highlights/álbuns), Replay card anual, fonte Inter, responsividade, TalkBack.

## Ponto de retomada

1. Validar em aparelho a replicação contínua entre dois dispositivos, incluindo sugestões repetidas e edição concorrente de playlists. Continuar com sessão limitada e sem expor credenciais.
2. Discord Android/Windows, login de reprodução YTM e downloads permanentes dependem respectivamente de transporte/SDK suportado, contrato autenticado e capacidade explícita de download. Assinatura oficial depende de credenciais reais. O aceite funcional depende de autorização para executar a validação.
