use std::sync::mpsc::{Receiver, RecvError};

/// Cada posição é registrada antes de iniciar seu processamento. Os resultados
/// podem chegar em paralelo, mas só são consumidos na ordem de registro.
/// Um produtor encerrado sem resultado libera sua posição com erro.
pub(crate) fn consume_in_order<C, T>(
    queue: Receiver<(C, Receiver<T>)>,
    mut consume: impl FnMut(C, Result<T, RecvError>),
) {
    for (context, result) in queue {
        consume(context, result.recv());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    #[test]
    fn resultados_invertidos_sao_inseridos_na_ordem_original() {
        let (queue_tx, queue_rx) = channel();
        let (inserted_tx, inserted_rx) = channel();
        let worker = std::thread::spawn(move || {
            consume_in_order(queue_rx, |id, result| {
                inserted_tx.send((id, result.unwrap())).unwrap();
            });
        });
        let (first_tx, first_rx) = channel();
        let (second_tx, second_rx) = channel();
        queue_tx.send((1, first_rx)).unwrap();
        queue_tx.send((2, second_rx)).unwrap();

        // O segundo termina enquanto o primeiro ainda está processando.
        second_tx.send("segundo").unwrap();
        assert!(inserted_rx.recv_timeout(Duration::from_millis(50)).is_err());
        first_tx.send("primeiro").unwrap();
        drop(queue_tx);
        assert_eq!(
            inserted_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            (1, "primeiro")
        );
        assert_eq!(
            inserted_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            (2, "segundo")
        );
        worker.join().unwrap();
    }

    #[test]
    fn falhas_de_processamento_e_colagem_nao_bloqueiam_os_seguintes() {
        let (queue_tx, queue_rx) = channel();
        let (finished_tx, finished_rx) = channel();
        let worker = std::thread::spawn(move || {
            consume_in_order(queue_rx, |id, result| {
                let result = result
                    .map_err(|_| "interrompido")
                    .and_then(|result: Result<&str, &str>| result)
                    .and_then(|text| {
                        if text == "falha ao colar" {
                            Err("clipboard")
                        } else {
                            Ok(text)
                        }
                    });
                finished_tx.send((id, result)).unwrap();
            });
        });
        let mut producers = Vec::new();
        for id in 0..4 {
            let (tx, rx) = channel();
            queue_tx.send((id, rx)).unwrap();
            producers.push(tx);
        }
        // Enfileirar continua possível enquanto a primeira posição espera.
        producers.pop().unwrap().send(Ok("último")).unwrap();
        producers.pop().unwrap().send(Ok("falha ao colar")).unwrap();
        drop(producers.pop().unwrap()); // worker encerrado sem enviar resultado
        producers
            .pop()
            .unwrap()
            .send(Err("transcrição falhou"))
            .unwrap();
        drop(queue_tx);
        for expected in [
            (0, Err("transcrição falhou")),
            (1, Err("interrompido")),
            (2, Err("clipboard")),
            (3, Ok("último")),
        ] {
            assert_eq!(
                finished_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
                expected
            );
        }
        worker.join().unwrap();
    }
}
