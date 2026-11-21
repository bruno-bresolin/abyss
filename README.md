# Abyss

Nova implementação do explorador Mandelbrot em **Rust + SDL3**. O código C++ do
repositório não é usado pelo aplicativo Rust.

O objetivo é separar precisão matemática, tempo de cálculo e apresentação:
a câmera continua animada enquanto uma imagem completa substitui a anterior.
Para um vídeo com cadência garantida, o modo offline espera cada quadro e só
então o grava. O tempo de renderização não altera a velocidade do vídeo.

## Implementado

- Câmera MPFR/Rug de 1280 bits; posições em strings decimais.
- Magnificação de `10^-1` a **`10^280`**; esse limite é explícito, não infinito.
- Referência em precisão adaptativa, com 192 bits de margem e blocos de 128 bits.
- Seleção automática do cálculo direto `f64` em vistas rasas; perturbação, rebasing
  e BLA hierárquica em vistas profundas, com domínio de validade.
- Normas com `hypot`, evitando elevar deltas como `1e-220` ao quadrado para compará-los.
- Detecção de cancelamento numérico severo e recálculo independente em MPFR.
- Pool Rayon persistente, blocos de linhas dinamicamente distribuídos e cancelamento cooperativo.
- Solicitação imutável; no máximo uma câmera esperando e dois frames na fila.
- Passes em 1/8, 1/4, 1/2 e resolução completa. Nenhum frame parcial é publicado.
- GPU via renderer SDL3: texturas, filtragem linear, reprojeção e transição de 160 ms.
- Cache visual em RAM com orçamento e cache de uma órbita reutilizável.
- Zoom exponencial, zoom ancorado no cursor, pan e movimento automático.
- Cor contínua com paleta estável entre frames; sem histograma variável por tile/frame.
- PNG offline com quatro amostras espaciais por pixel, combinadas em luz linear aproximada.
- Filmes com amostragem temporal, shutter configurável, sequência PNG e script FFmpeg.
- `#![forbid(unsafe_code)]` no código do aplicativo. SDL e GMP/MPFR são dependências nativas
  encapsuladas pelos seus bindings; isso não torna suas implementações C formalmente seguras.

O SDL3 compõe a imagem e informa o backend escolhido no terminal. Com a feature
opcional `gpu`, o explorador também pode calcular órbitas em uma GPU Intel,
AMD ou NVIDIA. A CPU aplica a paleta e continua responsável pelo cálculo de zoom profundo
e pelos modos offline.

## Compilar

Rust estável recente com edição 2024 (Rust 1.88 ou superior; dependências travadas
em `Cargo.lock` podem requerer versão mais recente), compilador C/C++, `make`,
`m4`, CMake e dependências de janela da SDL3.
GMP/MPFR são compilados pelo Rug se necessário.

Em Debian/Ubuntu, instale as dependências de desenvolvimento compatíveis com sua versão:

```sh
sudo apt-get install build-essential m4 cmake pkg-config \
  libx11-dev libxext-dev libxrandr-dev libxcursor-dev libxi-dev \
  libxfixes-dev libxss-dev libxkbcommon-dev libwayland-dev \
  libegl-dev libgl-dev
```

Dentro deste diretório:

```sh
# Compila SDL3 a partir dos fontes e abre o explorador.
cargo run --locked --release --features bundled-sdl -- explore

# Se a SDL3 já estiver instalada e acessível ao linker:
cargo run --locked --release -- explore

# Núcleo offline, sem SDL e sem servidor gráfico:
cargo build --locked --release --no-default-features
```

Consulte também as dependências da sua plataforma na documentação oficial SDL3.
A feature de console Unix usada no teste headless não deve substituir as
bibliotecas de janela em uma instalação desktop.

## Explorar

```sh
cargo run --locked --release --features bundled-sdl -- \
  --width 1280 --height 720 --iterations 12000 --memory-mb 1024 \
  --location examples/seahorse.json explore
```

### Cálculo em GPU Intel, AMD ou NVIDIA

```sh
cargo run --locked --release --features gpu,bundled-sdl -- --gpu intel explore
cargo run --locked --release --features gpu,bundled-sdl -- --gpu amd explore
cargo run --locked --release --features gpu,bundled-sdl -- --gpu nvidia explore
```

A placa selecionada precisa oferecer Vulkan e cálculo em precisão dupla
(`shaderFloat64`). Se ela não aparecer e houver pacotes Vulkan ausentes, o
Abyss mostra o comando e pede confirmação no terminal antes de executá-lo
com `sudo`. A instalação guiada usa `apt-get` no Debian e Ubuntu e `dnf`
no Fedora. Pressione Enter ou responda `n` para continuar com a CPU.
Pacotes Mesa Vulkan de cada distribuição: [Debian](https://packages.debian.org/stable/mesa-vulkan-drivers),
[Ubuntu](https://packages.ubuntu.com/noble/mesa-vulkan-drivers) e
[Fedora](https://packages.fedoraproject.org/pkgs/mesa/mesa-vulkan-drivers/).

Em placas NVIDIA, o Abyss usa o driver Vulkan disponível no sistema, desde que
ele exponha `shaderFloat64` para a placa. A instalação guiada oferece os pacotes
do carregador Vulkan e do Mesa. A instalação do driver proprietário NVIDIA é
gerenciada separadamente pela distribuição. Consulte a
[documentação do driver NVIDIA](https://download.nvidia.com/XFree86/Linux-x86_64/615.78.08/README/installedcomponents.html)
e a [documentação do Mesa NVK](https://docs.mesa3d.org/drivers/nvk.html).

O explorador volta ao cálculo na CPU se a placa estiver indisponível ou houver
falha no Vulkan. O caminho GPU é usado somente quando o tamanho do pixel
é de pelo menos `1e-9` no plano complexo e o limite de iterações é de até
`16384`. Isso mantém as vistas profundas no caminho de alta precisão.
Os comandos `render` e `film` continuam usando a CPU.

| Controle | Ação |
| --- | --- |
| Roda do mouse | Zoom suave, preservando o ponto sob o cursor |
| Arrastar com botão esquerdo | Pan |
| Clique duplo | Centralizar no ponto e aproximar |
| Espaço | Iniciar/parar movimento automático em direção ao centro |
| `+` / `-` | Velocidade do movimento, em décadas por segundo |
| `S` | Salvar coordenadas precisas em `location-<timestamp>.json` |
| `R` | Voltar à câmera inicial |
| `H` | Mostrar/ocultar controles |
| Esc | Sair |

O movimento automático limita sua distância à última solicitação renderizada.
Se o cálculo atrasar, o movimento desacelera em vez de ampliar indefinidamente
uma prévia sem detalhes. Para reprodução com duração e FPS fixos, use `film`.

## Uma imagem acima de 10^200

```sh
cargo run --locked --release --no-default-features -- \
  --width 640 --height 480 --iterations 4096 \
  --location examples/tip-220.json render --output tip-220.png
```

Esse preset usa a ponta exata `-2 + 0i`, em `10^220`, como calibração numérica.
Não é uma localização artística com espirais: sua geometria local é simples.
Uma localização interessante nessa profundidade exige coordenadas com centenas
de dígitos apropriados. Aumentar o zoom de um centro fornecido com apenas 15
dígitos não recupera os dígitos ausentes nem encontra novos detalhes por si só.

Para desativar BLA e comparar o caminho básico, passe `--no-bla` antes de `render`.
Arquivos de saída existentes não são sobrescritos.

## Renderizar um filme

```sh
cargo run --locked --release --no-default-features -- \
  --width 1920 --height 1080 --iterations 16000 --memory-mb 2048 \
  film --config examples/film.json --output frames

cd frames
sh encode.sh
```

O exemplo tem 20 segundos a 60 FPS e vai de `10^0` a `10^9`.
O script requer FFmpeg com libx264 e produz `zoom.mp4`, sem sobrescrever arquivos.
Todos os frames são calculados antes da codificação; essa configuração pode
levar bastante tempo. Para uma prévia rápida, reduza resolução, duração e amostras.

Formato da trajetória:

```json
{
  "location": {
    "real": "-0.743643887037151",
    "imag": "0.131825904205330",
    "log10_zoom": 0.0
  },
  "end_log10_zoom": 9.0,
  "seconds": 20.0,
  "fps": 60,
  "ease": true,
  "temporal_samples": 2,
  "shutter": 0.5
}
```

O centro fica fixo. `ease: false` dá velocidade constante no logaritmo do zoom;
`ease: true` suaviza entrada e saída. `temporal_samples` aceita 1 a 8 amostras.
`shutter` vai de 0 a 1 frame. Os limites da trajetória são respeitados.
O diretório de destino precisa estar vazio; sequências interrompidas ficam
preservadas para inspeção, mas retomada automática ainda não foi implementada.

## Ausência de buracos de renderização

Workers nunca escrevem em texturas nem em buffers que a UI esteja lendo.
Cada frame tem dono, câmera, resolução e geração. Resultados obsoletos são
rejeitados antes de chegarem à apresentação. Uma fila cheia não impede cancelar
ou encerrar o aplicativo.

Durante a navegação, imagens anteriores são reprojetadas. Uma prévia transformada
é temporária e pode ficar desfocada. Ao diminuir o zoom ou deslocar para fora da
cobertura do cache, a área recém-exposta usa o fundo da interface até a próxima
prévia completa. Não há pixels não inicializados ou tiles pintados de preto para
indicar que estão carregando. A abertura mostra um estado de preparação enquanto
a primeira referência é calculada.

Separadamente, `Unresolved` significa que a órbita não escapou dentro do limite
de iterações. Ela recebe azul muito escuro, não é prova de interior. Se uma região
ficar lisa/escura, aumente `--iterations` e confira a localização. Glitches
numéricos detectados são recalculados em MPFR, não tratados como interior.

BLA usa limites conservadores e reparos, mas não aritmética intervalar certificada.
Os testes cobrem casos concretos; não constituem prova de ausência universal de
artefatos. Não há promessa de imagem inédita em 60 FPS em qualquer ponto profundo.

## Arquitetura

| Arquivo | Responsabilidade |
| --- | --- |
| `src/camera.rs` | Coordenadas precisas, mapeamento e reprojeção |
| `src/math.rs` | Complexos, estados de pixel e oráculo MPFR |
| `src/engine.rs` | Referência, cache, BLA, perturbação, rebasing e reparos |
| `src/scheduler.rs` | Pool, gerações, filas limitadas e cancelamento |
| `src/desktop.rs` | Eventos SDL3 e composição de texturas |
| `src/gpu.rs`, `src/mandelbrot.wgsl` | Cálculo opcional de órbitas diretas em Vulkan FP64 |
| `src/palette.rs` | Paleta contínua e estável |
| `src/film.rs` | PNG, trajetória, amostragem temporal e codificação |
| `tests/correctness.rs` | Precisão, comparação MPFR e concorrência |

`--memory-mb` faz uma validação conservadora das dimensões e iterações e limita
o cache visual. Não é um limite rígido de RSS do processo nem de VRAM do driver.
O sistema operacional e a SDL mantêm suas próprias alocações. `--threads` permite
reservar núcleos para o desktop; o padrão deixa até dois núcleos livres.

A referência usa índices separados da iteração global do pixel, pois rebasing
reinicia apenas a posição na referência. A árvore BLA começa na iteração 1,
compõe transformações afins e seus raios, e só aplica saltos válidos dentro do
orçamento restante. Blocos próximos ao escape são desabilitados. A referência
reutiliza sua precisão apenas enquanto ela for suficiente para a câmera atual.

## Verificação

```sh
cargo fmt --check
cargo test --locked --features gpu,bundled-sdl
cargo test --locked --no-default-features
cargo clippy --locked --no-default-features --all-targets -- -D warnings
```

Os testes incluem coordenadas em `10^220`, comparação de iteração e cor suave
contra MPFR em regiões distintas, execução real de BLA, reparo por cancelamento
numérico, bursts de navegação, integridade de frames e encerramento com fila cheia.

O aplicativo foi compilado e testado com driver SDL3 dummy/software no ambiente
de desenvolvimento. Isso valida o ciclo SDL, não desempenho de GPU física.

## Referências matemáticas e APIs

- https://mathr.co.uk/web/deep-zoom.html — perturbação, rebasing e BLA.
- https://mathr.co.uk/blog/2021-05-14_deep_zoom_theory_and_practice.html — precisão e rescaling.
- https://docs.rs/rug/1.30.0/rug/struct.Float.html
- https://docs.rs/sdl3/0.22.1/sdl3/render/index.html
- https://wiki.libsdl.org/SDL3/README-linux

A implementação atual mantém deltas em `f64` e usa MPFR como reparo. Rescaling
com expoente estendido, estimativa de distância,
busca automática de núcleos e trajetórias com múltiplos centros são extensões
futuras, não funcionalidades simuladas ou declaradas prontas.
