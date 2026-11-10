# Contributing to Takeshot

<p align="center">
  <strong><a href="#english">English</a></strong> | <strong><a href="#portugues">Português</a></strong>
</p>

---

<a id="english"></a>

Contributions are welcome, from people and from AI agents. This guide takes you from an idea to a merged pull request. [AGENTS.md](AGENTS.md) holds the rules of the code itself (the layout, the interface rules, how objects and the resident work), and it applies to both.

## Before you start

- **A bug:** open an issue with the steps, what you expected, what happened and your system. Add a screenshot for anything on screen.
- **A feature or a change in behaviour:** open an issue first and describe it from the user's side. Takeshot's look and interaction are designed in [design/takeshot-ui.fig](design/takeshot-ui.fig) (open it in OpenPencil or Figma), so a visible change starts there.
- **One topic per pull request:** several small pull requests get reviewed faster than one large one.

## Set up

1. Install Rust: [rust-toolchain.toml](rust-toolchain.toml) picks the toolchain (MSVC on Windows). On Linux, also install the libraries listed under [Building](README.md#building) in the README.
2. Run `git config core.hooksPath .githooks` once. From then on, every push that touches the Rust build runs `scripts/check.sh` first: fmt, clippy and the tests on your machine, then the same on Linux in a container, plus clippy for macOS. It needs Docker, because code behind `#[cfg(...)]` for another system only compiles there.
3. Run `mkdir -p shots`. That folder is git-ignored and holds logs and screenshots.

## Make the change

- Create a branch named `feat/<topic>` or `fix/<topic>` from `main`.
- Write commit messages in English, as [Conventional Commits](https://www.conventionalcommits.org/): `feat: …`, `fix: …`, `docs: …`.
- Follow the layout and the interface rules in [AGENTS.md](AGENTS.md).
- Every user-facing string is a `Msg` key in `src/ui/i18n.rs`, with English and Portuguese text.
- A visible change has to look right in all four looks: TakeUI and Coral UI, each in light and dark.
  - All screens at once: run `scripts/screenshots.sh` after `cargo build --release`.
  - One screen: `takeshot --screenshot out.png --page … --theme … --mode …`.
  - Pictures for the repository or a pull request come from the sample, never from your own captures.
- New code, assets and values are your own work or come from openly licensed sources listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Check before you push

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test > shots/test.log 2>&1
```

All three must come back clean: fmt prints nothing, clippy reports no warnings (existing ones included), and the tests pass. Send the test output to a file and read it there, one run at a time.

## Open the pull request

- Fill in the [template](.github/pull_request_template.md): what changes for the person using Takeshot, a screenshot of each look you changed, and the checks.
- Link each issue with its own `Closes #N` line. The issue closes when the pull request merges.
- CI runs fmt, clippy and the tests on Windows, macOS and Linux.
  - It also builds the seven installers when the pull request touches how they are made (`Cargo.toml`, `Cargo.lock`, `build.rs`, `packaging/`, `assets/brand/`, `.github/`).
  - A new push cancels the run before it.
  - On a first contribution from a fork, a maintainer approves the run.
- Pull requests are squash-merged, so the title becomes the commit message. Write it as a Conventional Commit too.

## Releases

Only maintainers release, so a contributor's pull request never bumps `version`.

1. A pull request raises `version` in `Cargo.toml`.
2. After it merges, a maintainer runs Actions > release > Run workflow on `main`. The workflow tests, builds the installers, tags `vX.Y.Z` and publishes the GitHub Release.

The workflow refuses a version that is already out, and nobody pushes tags by hand.

## License

Takeshot is licensed under the [GNU Affero General Public License v3.0 or later](LICENSE). By opening a pull request, you agree that your contribution is licensed under the same terms.

---

<a id="portugues"></a>

# Contribuindo com o Takeshot

Contribuições são bem-vindas, de pessoas e de agentes de IA. Este guia leva você de uma ideia até um pull request com merge. O [AGENTS.md](AGENTS.md) tem as regras do código em si (a estrutura, as regras de interface, como funcionam os objetos e o residente) e vale para os dois.

## Antes de começar

- **Um bug:** abra uma issue com os passos, o que você esperava, o que aconteceu e o seu sistema. Mande uma captura de tela se o problema aparece na tela.
- **Uma funcionalidade ou mudança de comportamento:** abra uma issue antes e descreva do ponto de vista de quem usa. O visual e a interação do Takeshot são desenhados em [design/takeshot-ui.fig](design/takeshot-ui.fig) (abra no OpenPencil ou no Figma), então uma mudança visível começa por lá.
- **Um assunto por pull request:** vários PRs pequenos são revisados mais rápido que um grande.

## Preparar o ambiente

1. Instale o Rust. O [rust-toolchain.toml](rust-toolchain.toml) escolhe o toolchain (MSVC no Windows). No Linux, instale também as bibliotecas da seção [Compilar](README.md#compilar) do README.
2. Rode `git config core.hooksPath .githooks` uma vez. A partir daí, todo push que mexe no build Rust roda antes o `scripts/check.sh`: fmt, clippy e testes na sua máquina, depois o mesmo no Linux num container, mais o clippy do macOS. Ele precisa do Docker, porque código atrás de `#[cfg(...)]` de outro sistema só compila lá.
3. Rode `mkdir -p shots`. Essa pasta fica fora do git e guarda logs e capturas.

## Fazer a mudança

- Crie uma branch com o nome `feat/<tema>` ou `fix/<tema>` a partir da `main`.
- Escreva as mensagens de commit em inglês, em [Conventional Commits](https://www.conventionalcommits.org/): `feat: …`, `fix: …`, `docs: …`.
- Siga a estrutura e as regras de interface do [AGENTS.md](AGENTS.md).
- Todo texto da interface é uma chave `Msg` em `src/ui/i18n.rs`, em inglês e português.
- Uma mudança visível precisa ficar certa nos quatro visuais: TakeUI e Coral UI, cada um em claro e escuro.
  - Todas as telas de uma vez: rode `scripts/screenshots.sh` depois de `cargo build --release`.
  - Uma tela: `takeshot --screenshot saida.png --page … --theme … --mode …`.
  - Imagens para o repositório ou para um PR saem da amostra, nunca das suas capturas.
- Código, assets e valores novos são trabalho seu ou vêm de fontes com licença aberta, listadas em [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Verificar antes do push

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test > shots/test.log 2>&1
```

Os três precisam terminar limpos: o fmt não imprime nada, o clippy não mostra nenhum warning (inclusive os que já existiam) e os testes passam. Mande a saída dos testes para um arquivo e leia lá, uma execução por vez.

## Abrir o pull request

- Preencha o [template](.github/pull_request_template.md): o que muda para quem usa o Takeshot, uma captura de cada visual que você mudou e as verificações.
- Ligue cada issue com uma linha `Closes #N` própria. A issue fecha quando o PR entra.
- O CI roda fmt, clippy e testes no Windows, no macOS e no Linux.
  - Também gera os sete instaladores quando o PR mexe em como eles são feitos (`Cargo.toml`, `Cargo.lock`, `build.rs`, `packaging/`, `assets/brand/`, `.github/`).
  - Um push novo cancela o run anterior.
  - Na primeira contribuição vinda de um fork, um mantenedor aprova o run.
- Os PRs entram por squash, então o título vira a mensagem do commit. Escreva o título em Conventional Commits também.

## Releases

Só os mantenedores publicam releases, então o PR de quem contribui nunca sobe a `version`.

1. Um PR sobe a `version` no `Cargo.toml`.
2. Depois do merge, um mantenedor roda Actions > release > Run workflow na `main`. O workflow testa, gera os instaladores, cria a tag `vX.Y.Z` e publica a release no GitHub.

O workflow recusa uma versão que já saiu, e ninguém dá push em tag à mão.

## Licença

O Takeshot é licenciado sob a [GNU Affero General Public License v3.0 ou posterior](LICENSE). Ao abrir um pull request, você concorda que a sua contribuição fica sob os mesmos termos.
