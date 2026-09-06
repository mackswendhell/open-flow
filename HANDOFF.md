# Handoff — sugestões de melhoria

Data: 2026-09-06

Avaliação do código do Open Flow. Item 1 implementado em 2026-09-06; os demais
continuam pendentes. Próxima prioridade: confiabilidade do salvamento.

## 1. Garantir a ordem dos ditados — concluído

**Implementado em 2026-09-06:** cada ditado válido reserva sua posição antes do
processamento. Um consumidor único aguarda os resultados nessa ordem e faz as
colagens, enquanto captura, transcrição e revisão continuam independentes.
Falhas de processamento, produtores encerrados sem resultado e erros de colagem
liberam a posição para os seguintes. O último texto recuperável pela bandeja é
atualizado na ordem das tentativas de inserção. Implementação comum a Windows e
macOS em `lib.rs` e `insertion_queue.rs`.

**Verificação da correção:** `cargo test --lib --locked --offline` passou no macOS
(10 testes), incluindo conclusão invertida e continuidade após falhas. Build de
produção macOS gerado com `npm run tauri build -- --bundles app`, instalado em
`/Applications/OpenFlow.app` e reiniciado em 2026-09-06. Ainda pendem teste físico
de ditado/colagem e validação no Windows; sem nova release.

**Situação original:** cada gravação é processada em uma thread independente em
`app/src-tauri/src/lib.rs`, função `spawn_pipeline()`. O `insert_lock` impede
inserções simultâneas, mas não garante a ordem das gravações. Se o segundo ditado
terminar de processar antes do primeiro, os textos podem ser colados fora de ordem.

**Sugestão:** identificar cada ditado e ordenar sua inserção em uma fila, mantendo
a captura do próximo áudio disponível durante o processamento.

**Validar:** dois ditados com tempos de processamento invertidos devem ser
inseridos na ordem original. Uma falha no primeiro não deve bloquear os seguintes.

## 2. Completar o fallback da transcrição local

**Situação:** em `stt_provider()`, no mesmo arquivo Rust, a Groq assume quando
`ensure_sidecar()` falha, se o provedor selecionado for local e houver chave Groq.
Se o sidecar ficar pronto, mas `transcribe()` falhar, o erro é devolvido sem essa
tentativa de recuperação.

**Sugestão:** abranger também falhas da chamada de transcrição local no fallback,
respeitando a configuração de provedores e a disponibilidade da chave.

**Validar:** sidecar saudável seguido de erro de transcrição deve permitir a
tentativa na Groq; sem chave, devolver erro claro, sem repetir indefinidamente.

## 3. Tornar o salvamento mais confiável — prioridade alta

**Situação:** em `app/src/App.tsx`, o salvamento automático de `SettingsApp()` não
trata a rejeição de `invoke("save_settings")`, podendo deixar a interface em
“salvando...” indefinidamente. No Rust, `persist_settings()` sobrescreve diretamente
o `settings.json`, deixando margem para arquivo incompleto se a escrita for interrompida.

**Sugestão:** mostrar erro de salvamento e permitir nova tentativa. Gravar primeiro
em arquivo temporário e substituir o destino de forma atômica, com implementação
compatível com Windows e macOS. Preservar o tratamento de chaves e a tolerância a BOM.

**Validar:** falha de escrita deve manter a configuração anterior íntegra e gerar
feedback na interface. Uma nova tentativa bem-sucedida deve atualizar o estado para
“salvo”. Alterações rápidas não devem resultar em configurações antigas prevalecendo.

## 4. Preservar melhor a área de transferência

**Situação:** `insert_text()`, em `app/src-tauri/src/lib.rs`, guarda somente texto
com `get_text()`. Imagens e outros formatos não são restaurados. A restauração do
backup após a colagem também pode sobrescrever algo que o usuário acabou de copiar.

**Sugestão:** avaliar preservação dos formatos disponíveis por plataforma e
restaurar o backup somente se a área de transferência ainda contiver o conteúdo
colocado pelo app. Manter a recuperação do último ditado pela bandeja.

**Validar:** clipboard com texto e imagem; cópia feita pelo usuário durante a
inserção; falha no atalho de colar. Conferir o comportamento nos dois sistemas.

## 5. Avisar quando o texto sair sem revisão

**Situação:** em `spawn_pipeline()`, uma falha de `rewrite()` faz o app usar a
transcrição bruta e registrar o motivo apenas no console. O usuário pode receber
texto sem o tratamento do perfil sem entender o motivo.

**Sugestão:** exibir aviso discreto quando houver falha de revisão, preservando a
inserção do texto disponível. Distinguir falha de uma configuração sem chave Gemini.

**Validar:** erro ou timeout na revisão deve inserir o texto bruto e informar o
ocorrido; uma revisão bem-sucedida não deve mostrar aviso.

## Manutenção posterior

Separar gradualmente `app/src-tauri/src/lib.rs` em módulos de gravação,
provedores, configurações e inserção, preservando as interfaces da camada
`platform/`. Fazer essa reorganização depois das correções prioritárias, com
testes dos comportamentos afetados.

## Verificações realizadas na avaliação

- Referências remotas atualizadas com `git fetch`; a branch estava sincronizada
  com `origin/main`.
- `npm run build`, em `app/`: passou.
- `cargo test --lib --locked --offline`, em `app/src-tauri/`: oito testes passaram
  no macOS.
- Não houve teste físico de ditado, teste no Windows ou reprodução prática dos
  cenários acima. Os achados vieram da leitura do código.
- Nenhum arquivo-fonte foi alterado na avaliação.

## Cuidados para a próxima sessão

- Consultar `CLAUDE.md` e o README antes de implementar; há decisões e correções
  anteriores que precisam ser preservadas.
- Conferir as duas implementações de plataforma quando uma mudança as afetar.
- Não voltar a esconder/exibir a janela do overlay: o estado visual é controlado
  por eventos e pelo React, conforme as instruções do projeto.
- O grafo em `graphify-out/` é anterior ao código avaliado. Usar nomes de funções
  para localizar os pontos atuais; números de linha podem mudar.
