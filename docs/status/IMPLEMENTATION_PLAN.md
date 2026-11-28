# Kazzu — Escopo definitivo de implementação

Este documento substitui o master prompt e o checklist que não foram encontrados. Todas as fases abaixo estão previamente autorizadas.

Continue a partir das alterações já realizadas. Não reinicie o projeto, não repita a auditoria e não descarte o trabalho existente.

## Fase 1 — Backend Android

- [ ] Reativar o backend Android.

- [ ] Criar bootstrap Android específico para a UI v6.

- [ ] Conectar Engine, Library, Config, Controller e Worker.

- [ ] Criar AppBridge para conectar Rust e Slint.

- [ ] Remover dados fictícios dos caminhos de produção.

- [ ] Persistir configurações.

- [ ] Histórico de reprodução.

- [ ] Favoritos e curtidas.

- [ ] Pesquisa funcional.

- [ ] Cache inteligente.

- [ ] Migrar completamente a fila para o motor de áudio.

- [ ] Separar armazenamento persistente e cache descartável.

- [ ] Reaproveitar o SQLite existente.

## Fase 2 — Player

- [ ] Reprodução real de áudio.

- [ ] Play/pause, próxima e anterior.

- [ ] Volume e progresso.

- [ ] Shuffle e repeat.

- [ ] Fila funcional.

- [ ] Reordenação da fila.

- [ ] Gapless.

- [ ] Crossfade.

- [ ] Bit-perfect em dispositivos compatíveis.

- [ ] Seletor de saída de áudio.

- [ ] Timer de sono.

- [ ] Letras sincronizadas.

- [ ] Reprodução de videoclipes quando suportada.

- [ ] Reprodução em segundo plano.

- [ ] Foreground service.

- [ ] MediaSession.

- [ ] Notificação multimídia.

- [ ] Controle pela tela de bloqueio.

- [ ] Controle por Bluetooth.

- [ ] Gerenciamento de foco de áudio.

## Fase 3 — Músicas locais

- [ ] Seleção de pastas via SAF.

- [ ] Permissões persistentes.

- [ ] Escaneamento automático.

- [ ] Escaneamento incremental.

- [ ] Importação de arquivos.

- [ ] Leitura de metadados e capas.

- [ ] Edição de metadados.

- [ ] Organização por álbum, artista e gênero.

- [ ] Pesquisa local.

- [ ] Importação de playlists M3U.

- [ ] Detecção de duplicatas.

## Fase 4 — Provedores

- [ ] YouTube Music no modo visitante.

- [ ] YouTube Music com login.

- [ ] Pesquisa unificada.

- [ ] Importação de playlists entre serviços compatíveis.

- [ ] Alternância entre fontes de áudio.

- [ ] Seleção da qualidade de streaming.

- [ ] Gerenciamento de contas conectadas.

- [ ] Arquitetura extensível de provedores.

## Fase 5 — Biblioteca e downloads

- [ ] Criar playlists.

- [ ] Editar playlists.

- [ ] Excluir playlists.

- [ ] Capas personalizadas.

- [ ] Playlists inteligentes.

- [ ] Downloads autorizados.

- [ ] Reprodução offline.

- [ ] Gerenciador de downloads.

- [ ] Gerenciamento de armazenamento.

- [ ] Backup e restauração.

## Fase 6 — Recursos sociais

- [ ] Discord Rich Presence.

- [ ] Last.fm.

- [ ] Ouvir junto.

- [ ] Playlists colaborativas.

- [ ] Compartilhamento universal de músicas.

- [ ] Sincronização entre dispositivos.

- [ ] Controle remoto da reprodução.

Priorizar LAN, P2P e soluções descentralizadas quando tecnicamente viáveis.

## Fase 7 — Descoberta musical

- [ ] Descoberta de novos artistas.

- [ ] Mixes personalizados.

## Fase 8 — Estatísticas e Replay

- [ ] Tempo total de reprodução.

- [ ] Músicas mais ouvidas.

- [ ] Artistas mais ouvidos.

- [ ] Álbuns mais ouvidos.

- [ ] Gêneros favoritos.

- [ ] Estatísticas diárias, semanais e mensais.

- [ ] Retrospectiva anual estilo Spotify Wrapped.

- [ ] Exportação das estatísticas.

- [ ] Processamento local, sem telemetria.

## Fase 9 — UI/UX

- [ ] Preservar a estética atual da v6.

- [ ] Aprimorar o Liquid Glass.

- [ ] Temas dinâmicos baseados nas capas.

- [ ] Personalização avançada.

- [ ] Estados de carregamento.

- [ ] Gestos e animações funcionais.

- [ ] Acessibilidade e TalkBack.

- [ ] Aprimoramento da interface Android.

## Fase 10 — Segurança, desempenho e distribuição

- [ ] Zero telemetria própria.

- [ ] Armazenamento seguro de credenciais.

- [ ] Otimização de RAM.

- [ ] Otimização de CPU e GPU.

- [ ] Otimização de bateria.

- [ ] Correção de vazamentos de memória.

- [ ] Tratamento de falhas de rede.

- [ ] Atualização da documentação.

- [ ] Configuração do GitHub Actions.

- [ ] Preparação da assinatura oficial do APK.

## Explicitamente fora do escopo

- Nova arquitetura de banco de dados.

- Expansão do suporte Hi-Res/Lossless.

- Equalizador paramétrico avançado.

- Spotify, Tidal, Qobuz e SoundCloud como provedores de reprodução.

- ListenBrainz.

- Wear OS.

- MPRIS.

- Reconhecimento musical pelo microfone.

- Reconhecimento de músicas cantaroladas.

- Pesquisa por linguagem natural.

- Criação de playlists por IA.

- Recomendações inteligentes.

- Rádio baseada em músicas.

- Recomendações baseadas no histórico.

- Redesenho da interface desktop.

- Novos estados vazios e de erro como iniciativa visual independente.

- Novo modo sem conexão visual.

- Redesenho dos controles de redução de movimento e transparência.

- Criação de uma nova suíte de testes automatizados.

- Publicação no GitHub Releases.

Dependências técnicas indispensáveis podem ser implementadas, desde que justificadas. Não remova funcionalidades existentes que estejam fora do escopo.

## Decisões arquiteturais aprovadas

1. Bootstrap Android específico para a v6.

2. AppBridge separado do estado visual.

3. Reutilização do Controller existente.

4. Dados fictícios restritos aos previews.

5. Dados persistentes em getFilesDir() e cache em getCacheDir().

6. Reutilização de FTS5, YtmCatalog, Worker e cache existente.

7. Motor de áudio como única fonte de verdade da fila.

8. MediaSession e foreground service após a integração fundamental do backend.

9. Validação exclusivamente estática até autorização posterior.

## Restrições obrigatórias

- NÃO gerar build.

- NÃO executar cargo check ou testes.

- NÃO gerar APK ou release.

- NÃO disparar pipelines de compilação.

- NÃO comprometer a versão desktop.

- NÃO substituir a UI v6.

- NÃO introduzir Kotlin, Gradle, CMake ou Ninja sem autorização.

- NÃO adicionar telemetria.

- NÃO criar funcionalidades fictícias.

- NÃO inventar credenciais, keystores ou resultados de validação.

- Priorizar Rust, Slint, segurança e desempenho.

- Reutilizar o código existente.

- Respeitar as limitações dos provedores.

## Execução

Salve este documento no repositório como IMPLEMENTATION_PLAN.md e mantenha um IMPLEMENTATION_PROGRESS.md atualizado.

Continue imediatamente a implementação a partir do estado atual, incluindo a migração da cauda de faixas não resolvidas do Controller para a arquitetura de fila do motor.

Todas as dez fases estão autorizadas. Execute-as sequencialmente, sem interromper o trabalho para solicitar aprovações intermediárias ou realizar novos rounds de /grillme.

Quando houver impedimentos, registre-os e continue nas tarefas independentes.

Diferencie claramente funcionalidades pendentes, implementadas estaticamente, bloqueadas e aguardando validação.

Não encerre a execução apenas para relatar progresso parcial enquanto ainda houver tarefas executáveis dentro do escopo.
