# Verificação desta entrega

Data: 20 de novembro de 2026.

- Rust 1.100.0, SDL3 Rust bindings 0.22.1 e SDL nativa 3.4.20.
- Build otimizado do núcleo offline concluído.
- Build SDL3 estático concluído; ciclo de janela testado com driver dummy/software.
- Smoke test: quatro passes completos recebidos, estado final `awaiting=false`.
- 10 testes passaram, incluindo comparação com MPFR nas magnificações
  10^220 e 10^280, cancelamento numérico, BLA ativo, publicação de frames,
  fila cheia, exportação temporal de PNG e preservação de arquivos existentes.
- Clippy em todos os targets, incluindo desktop: sem avisos com `-D warnings`.
- Imagem `tip-220.png`: 128 x 96, 2048 iterações, quatro amostras espaciais,
  sem reparos e sem amostras pendentes nesse caso de calibração.
- Prévia `abyss-preview-60fps.mp4`: 480 x 300, 240 frames, 4 segundos,
  60 FPS, 2048 iterações, quatro amostras espaciais e duas temporais.
  Trajetória em `examples/preview-film.json`, do zoom 10^1 ao 10^5.

As verificações numéricas usam casos definidos e não certificam todas as
localizações possíveis. A ponta -2 é uma calibração exata e não uma região
artística complexa em 10^220. A prévia de vídeo demonstra a trajetória e
amostragem em zoom menor; não é uma gravação em tempo real de um zoom profundo.

Nenhum benchmark de GPU física foi realizado. Nesta implementação a GPU
apresenta e reprojeta texturas; as órbitas são calculadas na CPU. O modo offline
espera cada quadro, portanto o custo de cálculo não introduz pausas no vídeo.
