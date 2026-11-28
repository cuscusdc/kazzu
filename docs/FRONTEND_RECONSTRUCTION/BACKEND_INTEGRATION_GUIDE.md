# Integrar um serviço

1. Comece pelo estado no backend que confirma a operação. A existência de um item de catálogo prova que há metadados. A capacidade de tocar, baixar, obter letra e abrir a aplicação de origem depende de contratos distintos.
2. Converta o resultado em um modelo Rust da camada de apresentação. Use o identificador do provedor/serviço e da faixa ou visita. Preserve o identificador de ocorrência em filas, inclusive com faixas repetidas.
3. Encaminhe o evento à thread de interface pelo mecanismo de mensagens existente. Registre o callback em `app.rs`, publique pela `View` em `slint_view.rs` e exponha a capacidade somente quando ambos os sentidos estiverem ligados. Mantenha I/O no worker.
4. Ao mudar de faixa, serviço ou sessão, invalide dados anteriores, limpe mensagens e publique a nova disponibilidade antes do conteúdo. Em detalhes use a sequência existente. Cancelar um painel não equivale a cancelar automaticamente a operação de rede.
5. Para letras publique Loading, seguido de Ready com linhas, Empty ou Failed com mensagem. Limpe a linha ativa ao trocar de faixa. A apresentação Slint suporta consulta e nova tentativa; o fornecedor real e a sincronização precisam de implementação externa.
6. Teste estado inicial, sucesso, falha recuperável, ausência de capacidade, resposta tardia e repetição. A galeria e as fixtures de teste podem enviar esses eventos por View. Em produção use exclusivamente o adaptador real.

**Exemplo integrado de escrita.** Em Editar playlist, mover uma faixa envia posição visível e ID ao worker. O worker consulta a playlist unificada, verifica o item, usa reorder_playlist e publica PlaylistEdited. O controlador recarrega a visita corrente somente se a playlist ainda estiver em exibição. A biblioteca recebe um snapshot atualizado.

A matriz de entrega está em [progresso](IMPLEMENTATION_PROGRESS.md). As capacidades ainda sem fornecedor e o alcance dos testes estão em [limitações](KNOWN_LIMITATIONS.md) e [validação](../tests/TEST_REPORT.md).
