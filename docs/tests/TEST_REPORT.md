# Validação da reconstrução do frontend

| Verificação | Resultado observado | Evidência na árvore de build |
|---|---|---|
| Linux x86_64 | Workspace compilado em 7 min 52 s na revisão final. O executável iniciou com `--help`. | `target/frontend-final-build.log`, `target/debug/kazzu` |
| Linux AArch64 | Binário release compilado em 14 min 53 s. Inicialização com `--help` sob QEMU com as bibliotecas ARM64 do empacotamento local. | `target/frontend-linux-arm.log`, `target/frontend-arm-startup.log` |
| Android ARM64 | APK local de desenvolvimento com assinatura v2/v3, alinhamento ZIP de 16 KiB e SHA-256 conferidos. | `target/frontend-android.log`, `target/android/frontend-reconstruction/` |
| Design | 47 pares de cores avaliados pelo gerador de tokens, todos aprovados. | `target/frontend-tokens.log` |
| Renderização | Capturas por software em claro e escuro, incluindo player OLED em 844 × 390 e 1024 × 768. O executável desktop também produziu capturas com FemtoVG e acervo isolado. | `target/ui-snapshots/`, `target/frontend-snapshots-gpu/` |
| Windows | Toolchain indisponível no ambiente. | Sem binário validado |

**APK.** `kazzu-0.5.0-alpha.1-arm64-v8a.apk` · SHA-256 `4522ca68a9d772d0939e490cab3f0f142bf7d6a6becb9eb22f564e9343142048`.

**Fluxos exercitados.** A suíte do controlador cobre navegação entre detalhes e seções, restauração do contexto de reprodução, renomear e excluir playlist, mover e remover itens e rejeitar uma edição com índice obsoleto. O teste de interação Slint usa ponteiro e teclado para navegar, editar uma pesquisa, abrir e recolher o player, voltar da fila e fechar uma folha. Capas sintetizadas pertencem às fixtures.

Na primeira execução completa, o teste de interface tentou usar F como atalho com o campo de Pesquisa focado. O campo recebeu a letra. O teste foi corrigido para sair da Pesquisa pela sidebar antes de verificar o atalho de reprodução. A primeira rodada registrou 226 aprovados, uma falha e nove ignorados em `target/frontend-tests.log`. A repetição integral foi interrompida depois de recompilar o teste corrigido. O executável de renderização dessa compilação foi então executado separadamente e passou, conforme `target/frontend-ui-final.log`. No conjunto das rodadas, 227 testes distintos passaram e nove ficaram ignorados. A alteração entre rodadas atingiu exclusivamente o cenário de teste.

O APK desta revisão não foi instalado em aparelho físico. Login real, áudio/GPU em equipamento ARM64, TalkBack e IME físico permanecem sem verificação. Os arquivos com prefixo gl-mobile do ensaio FemtoVG usaram o shell Android no compositor desktop, que forneceu um framebuffer de 1920 × 1020. As dimensões exatas do ensaio de responsividade provêm do renderer de software.
