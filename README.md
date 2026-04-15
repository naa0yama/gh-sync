# gh-sync

![coverage](https://raw.githubusercontent.com/naa0yama/gh-sync/badges/coverage.svg)
![test execution time](https://raw.githubusercontent.com/naa0yama/gh-sync/badges/time.svg)

upstream テンプレートリポジトリから downstream リポジトリへファイルを同期する CLI ツール

## 概要

`gh-sync` は `boilerplate-rust` などのテンプレートリポジトリで管理されているファイルを、
downstream (fork) リポジトリへ pull 型で同期する Rust 製 CLI です。
`GITHUB_TOKEN` のみで動作し、GitHub App や PAT は不要です。

詳細は [`docs/specs/template-sync.ja.md`](docs/specs/template-sync.ja.md) を参照してください。

## 必要要件

- Docker
- Visual Studio Code
- VS Code Dev Containers 拡張機能

## セットアップ

1. リポジトリをクローン:

```bash
git clone <repository-url>
cd gh-sync
```

2. VS Code でプロジェクトを開く:

```bash
code .
```

3. VS Code のコマンドパレット (`Ctrl+Shift+P` / `Cmd+Shift+P`) から「Dev Containers: Reopen in Container」を選択

## GitHub Action として使う

`uses: naa0yama/gh-sync@<tag>` で下流リポジトリの CI に組み込めます。
マニフェストのスキーマ検証 → リポジトリ設定のドリフト検知 → ファイルのドリフト検知を順に実行します。

```yaml
# .github/workflows/gh-sync.yaml
name: gh-sync check
on:
  push:
    branches: [main]
  pull_request:
    types: [opened, synchronize, reopened]
  schedule:
    - cron: "0 18 * * *" # daily at 03:00 JST
  workflow_dispatch:

permissions: {}

jobs:
  gh-sync-check:
    name: gh-sync-check
    runs-on: ubuntu-latest
    timeout-minutes: 10
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@<sha> # vX.Y.Z
        with:
          persist-credentials: false
      - uses: naa0yama/gh-sync@v0.1.3
        with:
          token: ${{ secrets.GITHUB_TOKEN }}
```

`gh-sync init --with-workflow` を実行すると、このワークフロー雛形を `.github/workflows/gh-sync.yaml` として自動生成できます。

マニフェストを upstream リポジトリから取得して使う場合は `upstream-manifest` を指定します。
ローカルの `manifest` ファイルが存在すれば、同じ `path` のルールで local が上書き (local overlay) されます。

```yaml
- uses: naa0yama/gh-sync@v0.1.3
  with:
    token: ${{ secrets.GITHUB_TOKEN }}
    upstream-manifest: naa0yama/boilerplate-rust@main:.github/gh-sync/config.yaml
    # manifest: .github/gh-sync/config.yaml  # local overlay (省略可)
```

### inputs

| name                | required | default                       | 説明                                                                         |
| ------------------- | :------: | ----------------------------- | ---------------------------------------------------------------------------- |
| `token`             |   yes    | —                             | `gh` CLI 用トークン。`${{ secrets.GITHUB_TOKEN }}` を推奨                    |
| `version`           |    no    | `github.action_ref`           | ダウンロードするリリースタグ。SHA pin の場合は明示指定が必要                 |
| `manifest`          |    no    | `.github/gh-sync/config.yaml` | 同期設定ファイルのパス                                                       |
| `upstream-manifest` |    no    | —                             | upstream マニフェスト参照。`owner/repo@ref:path` 形式。local との merge も可 |

## 使い方

すべてのタスクは `mise run <task>` で実行します。

### 基本操作

```bash
mise run build            # デバッグビルド
mise run build:release    # リリースビルド
mise run test             # テスト実行
mise run test:watch       # TDD ウォッチモード
mise run test:doc         # ドキュメントテスト
```

### コード品質

```bash
mise run fmt              # フォーマット (cargo fmt + dprint)
mise run fmt:check        # フォーマットチェック
mise run clippy           # Lint
mise run clippy:strict    # Lint (warnings をエラー扱い)
mise run ast-grep         # ast-grep カスタムルールチェック
```

### コミット前チェック

```bash
mise run pre-commit       # clean:sweep + fmt:check + clippy:strict + ast-grep + lint:gh
```

## プロジェクト構造

```
.
├── .cargo/                     # Cargo 設定
│   └── config.toml
├── .devcontainer/              # Dev Container 設定
│   ├── devcontainer.json
│   ├── initializeCommand.sh
│   └── postStartCommand.sh
├── .githooks/                  # Git hooks (mise run 連携)
│   ├── commit-msg              # Conventional Commits 検証
│   ├── pre-commit              # コミット前チェック
│   └── pre-push                # プッシュ前チェック
├── .github/                    # GitHub Actions & 設定
│   ├── actions/                # カスタムアクション
│   ├── infra/                  # gh-infra マニフェスト (リポジトリ設定の宣言的管理)
│   ├── gh-sync/                # テンプレート同期設定
│   │   └── config.yaml         # 同期マニフェスト
│   ├── workflows/              # CI/CD ワークフロー
│   ├── labeler.yml
│   ├── project-config.json     # CI/リリース設定 (ビルドターゲット・タイムアウト等)
│   └── release.yml
├── .mise/                      # mise タスク定義
│   ├── tasks.toml              # 共通タスク定義 (boilerplate から管理)
│   └── overrides.toml          # プロジェクト固有のタスク上書き
├── .vscode/                    # VS Code 設定
│   ├── launch.json             # デバッグ設定
│   └── settings.json           # ワークスペース設定
├── ast-rules/                  # ast-grep プロジェクトルール
├── crates/                     # ワークスペースクレート (3クレート構成)
│   ├── gh-sync-manifest/       # 純粋データ型・スキーマ (I/O なし、Miri 対応)
│   │   ├── src/
│   │   │   ├── error.rs        # バリデーションエラー型
│   │   │   ├── manifest.rs     # マニフェストスキーマ・ロード・バリデーション
│   │   │   ├── strategy.rs     # 戦略結果型
│   │   │   └── lib.rs
│   │   └── Cargo.toml
│   ├── gh-sync-engine/         # ビジネスロジック・トレイト (外部バイナリ I/O なし、Miri 対応)
│   │   ├── src/
│   │   │   ├── diff.rs         # unified diff 生成
│   │   │   ├── mode/           # sync / validate / ci-check / patch-refresh モード
│   │   │   ├── output.rs       # ターミナル・PR 出力フォーマット
│   │   │   ├── repo/           # GhRepoClient トレイト・API 型
│   │   │   ├── strategy/       # 戦略実装 (replace / create_only / delete / patch)
│   │   │   ├── upstream.rs     # upstream フェッチャートレイト
│   │   │   └── lib.rs
│   │   └── Cargo.toml
│   └── gh-sync/                # CLI バイナリクレート (gh-sync-manifest + gh-sync-engine を利用)
│       ├── src/
│       │   ├── main.rs         # アプリケーションのエントリーポイント
│       │   ├── sync/           # テンプレート同期サブコマンド
│       │   │   └── runner.rs   # GhRunner トレイト (gh CLI 呼び出し抽象化)
│       │   └── init/           # init サブコマンド
│       ├── tests/
│       │   └── integration_test.rs  # 統合テスト
│       ├── build.rs            # ビルドスクリプト
│       └── Cargo.toml          # クレート設定
├── docs/                       # ドキュメント
│   └── specs/
│       └── template-sync.ja.md # 仕様書
├── .editorconfig               # エディター設定
├── .gitignore                  # Git 除外設定
├── .octocov.yml                # カバレッジレポート設定
├── .tagpr                      # タグ & リリース設定
├── action.yml                  # GitHub Action 定義 (naa0yama/gh-sync として利用可)
├── Cargo.lock                  # 依存関係のロックファイル
├── Cargo.toml                  # ワークスペース設定と共有依存関係
├── deny.toml                   # cargo-deny 設定
├── Dockerfile                  # Docker イメージ定義
├── dprint.jsonc                # Dprint フォーマッター設定
├── LICENSE                     # ライセンスファイル
├── mise.toml                   # ツール管理 (タスクは .mise/ を参照)
├── README.md                   # このファイル
├── renovate.json               # Renovate 自動依存関係更新設定
├── rust-toolchain.toml         # Rust toolchain バージョン固定
└── sgconfig.yml                # ast-grep 設定ファイル
```

## VSCode 拡張機能

このプロジェクトの Dev Containers には、Rust 開発を効率化する以下の拡張機能が含まれています:

### Rust 開発

- **[rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)** - Rust 言語サポート (コード補完、エラー検出、リファクタリング)
- **[CodeLLDB](https://marketplace.visualstudio.com/items?itemName=vadimcn.vscode-lldb)** - Rust プログラムのデバッグサポート
- **[Even Better TOML](https://marketplace.visualstudio.com/items?itemName=tamasfe.even-better-toml)** - Cargo.toml ファイルのシンタックスハイライトとバリデーション

### コード品質・フォーマット

- **[dprint](https://marketplace.visualstudio.com/items?itemName=dprint.dprint)** - 高速なコードフォーマッター (設定ファイル: `dprint.jsonc`)
- **[EditorConfig for VS Code](https://marketplace.visualstudio.com/items?itemName=EditorConfig.EditorConfig)** - エディター設定の統一
- **[Error Lens](https://marketplace.visualstudio.com/items?itemName=usernamehw.errorlens)** - エラーと警告をインラインで表示

### 開発支援

- **[Claude Code for VSCode](https://marketplace.visualstudio.com/items?itemName=Anthropic.claude-code)** - AI アシスタントによるコーディング支援
- **[indent-rainbow](https://marketplace.visualstudio.com/items?itemName=oderwat.indent-rainbow)** - インデントレベルを色分け表示
- **[Local History](https://marketplace.visualstudio.com/items?itemName=xyz.local-history)** - ファイルの変更履歴をローカルに保存

## ライセンス

このプロジェクトは [LICENSE](./LICENSE) ファイルに記載されているライセンスの下で公開されています。

### サードパーティライセンスについて

Dev Container の起動時に [OpenObserve Enterprise Edition](https://openobserve.ai/) が自動的にダウンロード・インストールされます。Enterprise 版は 200GB/Day のインジェストクォータ内であれば無料で利用できます。

OpenObserve Enterprise Edition は [EULA (End User License Agreement)](https://openobserve.ai/enterprise-license/) の下で提供されており、OSS 版 (AGPL-3.0) とはライセンスが異なります。

## 参考資料

- [The Rust Programming Language 日本語版](https://doc.rust-jp.rs/book-ja/)
- [Developing inside a Container](https://code.visualstudio.com/docs/devcontainers/containers)
- [Cargo Documentation](https://doc.rust-lang.org/cargo/)

## Troubleshooting

### デバッグ実行

```bash
RUST_LOG=trace RUST_BACKTRACE=1 cargo run -- sync --help
```
