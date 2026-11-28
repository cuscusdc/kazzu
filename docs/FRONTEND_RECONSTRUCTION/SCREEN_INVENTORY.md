# Destino das 44 superfícies

Atualização da interface do HTML registrada em [HTML_INTEGRATION.md](HTML_INTEGRATION.md). Ela acrescenta as paletas e a rota própria de Explorar ao Kazzu existente.

Sequência e nomes seguem o [diagnóstico original](../ui-audit/03_SCREEN_BY_SCREEN.md). As descrições identificam a implementação ou a dependência. A confirmação por execução está no [relatório de testes](../tests/TEST_REPORT.md).

| Nº | Superfície | Decisão e comportamento |
|---|---|---|
| 01 | Início Android | MobileHome fornece título e acesso a Ajustes; DiscoveryFeed monta as seções a partir dos modelos. |
| 02 | Explorar Android | Consolidada em Início; gêneros reais usam o ID de catálogo. A ilha tem três destinos. |
| 03 | Início desktop | DesktopHome compõe o feed editorial com títulos explícitos, estado offline, cache, erro e ação de recuperação. |
| 04 | Busca Android | Pesquisa mantém fonte, consulta e chips em carga, erro, vazio e resultados. O rail local considera a fonte ativa. |
| 05 | Busca desktop | Fonte e tipo são selecionáveis. Cabeçalho e resultados ocupam a coluna desktop. |
| 06 | Biblioteca Android | Resumo de escuta, acesso a coleções e filtros nativos; a seleção de Faixas abre diretamente as músicas. |
| 07 | Biblioteca desktop | Sidebar persistente; grade/lista, densidade e ordem alfabética por categoria. |
| 08 | Playlists da biblioteca | Criar e abrir reais; editor da playlist local permite renomear, reordenar, remover item e excluir com confirmação. |
| 09 | Álbuns da biblioteca | Grade/lista recebe todos os álbuns locais; a Home mantém sua própria seleção mais curta. |
| 10 | Faixas locais | Consulta ao worker existente; Faixas abre o detalhe tocável do acervo. |
| 11 | Artistas da biblioteca | Modelo real de artistas, ordenação A–Z/Z–A e rota do artista local. |
| 12 | Detalhe de artista | Discografia, faixas, sobre e conteúdo relacionado condicionados ao modelo; a pilha restaura visita anterior. |
| 13 | Detalhe de álbum | Detalhe usa faixa e capa reais e conserva o contexto de reprodução no retorno. |
| 14 | Detalhe de playlist | Editar aparece apenas quando DetailVm traz ID de playlist local. Referências Spotify são externas. |
| 15 | Curtidos | Coleção persistente pelo contrato existente; coração reflete o estado da faixa. |
| 16 | Todos os álbuns | A coleção continua acessível pelos atalhos; eliminada a segunda chamada dentro da lista de álbuns. |
| 17 | Coleção de músicas | Faixas têm IDs e contexto de ordem próprios. Falha de leitura agora chega com a sequência do pedido. |
| 18 | Detalhe de gênero | Gêneros vindos do catálogo consultam a rota correspondente; eliminadas buscas literais pelos chips fictícios. |
| 19 | Baixados | Retirada a entrada promocional da lista de coleções. Contrato de download depende do backend. |
| 20 | Mini-player Android | MobileMiniPlayer com capa real, título, artista, reprodução e próxima; separado da navegação. |
| 21 | Rodapé desktop amplo | DesktopNowBar fixo, ações e volume. Seek condicionado à duração. |
| 22 | Rodapé desktop compacto | DesktopCompactBar fixa e prioriza título; janela curta dispensa Próxima quando o espaço é insuficiente. |
| 23 | Player Android expandido | Palco sem Flickable, transporte ancorado, informações do áudio, arte real e duas colunas em tablet/paisagem. |
| 24 | Player desktop expandido | Palco sem rolagem do transporte; espaço restante define tamanho da capa e do console. |
| 25 | Fila dentro do player Android | Substituída pela QueueSheet única. Mesmos eventos de mover, remover e saltar. |
| 26 | Fila compartilhada | Atual visível sem próximas, confirmação antes de parar e limpar. Editar mantém o painel. |
| 27 | Letra Android | LyricsSheet preparada para linhas temporizadas ou estáticas e estados de requisição. Fornecedor ausente em produção. |
| 28 | Menu de faixa | Menu explícito e clique contextual onde há ações; retiradas chamadas promocionais a timer, download e letra. |
| 29 | Adicionar à playlist | Folha reutiliza playlists reais e segue para criação quando necessário. |
| 30 | Nova playlist | Nome e validação isolados. Seleção de pasta e criação usam folhas limitadas à área útil. |
| 31 | Importar Spotify | Importação de metadados por token, com explicação de reprodução externa, status e resultado persistentes. |
| 32 | Importar YouTube | Importação de playlists por token existente; sessão autenticada do Music depende de um adaptador. |
| 33 | Pasta Android | Listagem assíncrona no armazenamento acessível, invalidação da seleção anterior e recuperação na raiz. |
| 34 | Pasta desktop | Seletor nativo e caminho manual mantidos. Falha na seleção não deve reaproveitar um diretório anterior. |
| 35 | Ajustes Android | Categorias pesquisáveis; opções visuais atendem preferências do Android, sem atalhos de desktop na navegação principal. |
| 36 | Ajustes desktop | Preferências do shell desktop persistentes; mesmo contrato de tema e custo gráfico. |
| 37 | Geral | Modo visitante disponível desde o primeiro uso. Banner inicial explica as fontes reais. |
| 38 | Reprodução | Volume, rádio e qualidade por provedor; processamento indisponível é descrito no próprio grupo. |
| 39 | Aparência | Claro, escuro, automático e OLED; custo gráfico em quatro níveis, atmosfera e opções de acessibilidade. |
| 40 | Fontes e biblioteca | Gerenciamento de pastas, reindexação e importação com estado independente. |
| 41 | Privacidade | Confirmação de apagar histórico permanece até escolha explícita; alternativa Cancelar. |
| 42 | Atalhos | Atalhos globais bloqueados enquanto houver modal; Escape/Voltar atua na camada aberta. |
| 43 | Sobre e licenças | Retirada a declaração conflitante sobre a licença do aplicativo. Permanecem os créditos existentes das dependências. |
| 44 | Oferta de desempenho | Oferta de reduzir transparência preservada; modo Econômico também desliga desfoque. |

## Superfícies adicionais

- **Informações de reprodução** reúne origem, formato informado, detalhes, saída e volume. Ações de equalizador e troca de saída dependem de capacidades do motor.
- **Editar playlist** disponibiliza ações para a coleção local corrente, com confirmação de exclusão.
- **Ouvidas recentemente** lê até 200 faixas do histórico local real. A seção de recentes da Home também pode conter adições recentes e é rotulada separadamente.
- **Tablet Android** mantém a navegação móvel com ilha e mini-player centrados e usa capa e console lado a lado quando houver espaço.
- **Sessão e serviços opcionais** permanecem atrás do contrato OptionalServices. Login interativo, logout e letras só são apresentados com adaptador disponível.
