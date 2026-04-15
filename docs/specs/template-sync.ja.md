# Template Sync — 仕様書 v2

## 1. 概要

### 1.1 課題

`boilerplate-rust` はテンプレートリポジトリとして CI ワークフロー、lint 設定、devcontainer
設定などのインフラファイルを複数の downstream Rust プロジェクト(dtvmgr, chezmage,
dna-assistant, fterm)と共有している。

現在これらのファイルは `gh-infra` の File マニフェストで管理しているが、以下の限界がある:

- **テンプレート機能の不足**: Go テンプレートのプレースホルダ(`<% .Repo.Owner %>`)は
  メタデータのみ対応し、ファイル構造のカスタマイズができない
- **patch 未対応**: downstream がファイルの一部を変更する必要がある場合
  (例: `Cargo.toml` の workspace 設定)、表現する手段がない
- **未署名 commit**: `gh-infra` は GPG/SSH 署名なしで commit を作成するため、
  サプライチェーン検証に支障がある
- **404 エラー落ち**: upstream で削除されたファイルを `source: /dev/null` で宣言しても
  404 が返るとCLI がエラー終了し、後続ルールの実行が阻害される

### 1.2 解決策

`gh-sync sync` — `gh-sync` CLI のサブコマンドとして、upstream テンプレートリポジトリから
downstream(fork)リポジトリへファイルを同期する。

設計判断:

- **pull 型モデル**: downstream リポジトリが同期マニフェストを所有する。
  upstream リポジトリは downstream への書き込みアクセスを持たない
- **`GITHUB_TOKEN` のみ**: GitHub App のインストールや PAT の発行は不要。
  fork 自身の `GITHUB_TOKEN`(GitHub Actions でデフォルトで利用可能)で
  public な upstream リポジトリの読み取りと fork 内での PR 作成が可能
- **署名付き commit**: `gh-sync sync` はファイル書き込みのみ行い、commit は skill 経由で
  Claude が担当する。署名はユーザーの gitconfig 設定または GitHub Actions の
  commit API(自動 Verified)に委ねる
- **複数戦略**: ファイルの種類に応じて「完全コピー」から「プロジェクト固有の変更適用」
  まで異なるマージ方式が必要

### 1.3 信頼モデル

**脅威**: 侵害された upstream テンプレートリポジトリが悪意ある CI ワークフローや
ビルド設定を downstream プロジェクトに送り込む。

**緩和策**:

1. **fork 所有のマニフェスト**: downstream リポジトリが同期対象と方法を明示的に宣言する。
   攻撃者が upstream のみを変更しても新しい同期対象を追加することはできない
2. **PR ベースの配信**: 同期結果は PR として配信され、直接 push はしない。
   メンテナがマージ前に差分をレビューする
3. **署名付き commit**: 同期 commit は downstream リポジトリの ID で署名される
   (upstream ではない)。明確な出所を確立する
4. **戦略による制御**: `patch` 戦略により、upstream ファイルの特定部分のみを
   受け入れることが可能で、upstream 変更の影響範囲を縮小する

**保護しないもの**:

- 侵害された downstream マニフェスト(攻撃者が `.github/gh-sync/config.yaml` を
  変更できれば同期対象を制御できる)
- 受け入れた同期範囲内の悪意あるコンテンツ(`replace` でワークフローファイルを
  同期する場合、upstream ファイル全体が信頼される)

### 1.4 ユースケース

#### UC-1: 新規 downstream プロジェクトの完全インフラ同期

`boilerplate-rust` テンプレートから新プロジェクトを作成。マニフェストに全 CI ワークフロー、
lint 設定、devcontainer ファイルを `replace` 戦略でリスト。週次スケジュール同期で
全ファイルを最新に保つ。

#### UC-2: プロジェクト固有オーバーライド付き選択的同期

`dtvmgr` は CI ワークフローを共有するが、`Cargo.toml` の workspace 設定と
追加のリリースターゲットは独自。マニフェストでワークフローには `replace`、
`Cargo.toml` には `patch`、`project-config.json` には `create_only` を使用。

#### UC-3: CI での drift detection

PR の CI ジョブで `gh-sync sync --ci-check` を実行し、管理対象ファイルが期待状態から
乖離していないか検証。テンプレート管理すべきファイルへの意図しない手動編集を検出する。

#### UC-4: PR でのマニフェストバリデーション

`.github/gh-sync/config.yaml` が変更された際、CI ジョブで `gh-sync sync --validate` を実行し、
マニフェストが正しい形式であることをマージ前に検証。

### 1.5 スコープ

#### 目標

- GitHub リポジトリからの個別ファイル・ディレクトリの同期
- 4つの戦略: `replace`, `create_only`, `delete`, `patch`
- マニフェストバリデーション(スキーマ + 参照チェック)
- drift detection(ローカル状態と同期後の期待状態の比較)
- dry-run モード
- CI 連携向けの構造化出力

#### 非目標

- push 型モデル(upstream → downstream)
- コンフリクト解決 UI やインタラクティブマージ
- git 履歴の同期や upstream commit attribution の保存
- GitHub 以外のホスティング対応(GitLab, Bitbucket)
- PR の自動マージ(人間によるレビューが意図的)
- バイナリファイルの patch 処理

### 1.6 アーキテクチャ

`gh-sync` はワークスペース内の 3 クレートに分割されている。

```
gh-sync-manifest   — 純粋データ型・スキーマ (I/O なし、Miri 対応)
        ↓
gh-sync-engine     — ビジネスロジック・トレイト (外部バイナリ I/O なし、Miri 対応)
        ↓
gh-sync            — CLI バイナリ (GhRunner 実装・コマンドライン解析)
```

#### クレートの責務

| クレート           | 責務                                                                                   | I/O                                                    |
| ------------------ | -------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| `gh-sync-manifest` | マニフェストスキーマ型、YAML ロード、バリデーション                                    | YAML ファイル読み取りのみ                              |
| `gh-sync-engine`   | 戦略実装、sync/validate/ci-check/patch-refresh モード、出力フォーマット                | なし(`GhRepoClient` / `GhRunner` トレイト経由で抽象化) |
| `gh-sync`          | `GhRunner` トレイット本実装(`SystemGhRunner`)、`clap` コマンド解析、エントリーポイント | `gh` CLI 呼び出し・ファイルシステム書き込み            |

#### `GhRunner` トレイト

`crates/gh-sync/src/sync/runner.rs` で定義するトレイト。
`gh` CLI の呼び出しを抽象化し、テストでモック実装を注入できるようにする。

```rust
pub trait GhRunner: Send + Sync {
    fn run(&self, args: &[&str], stdin: Option<&[u8]>) -> anyhow::Result<GhOutput>;
}
```

本番環境では `SystemGhRunner` が `std::process::Command` で `gh` を起動する。
テストでは `MockGhRunner` を注入することで、`gh` CLI なしでユニットテストが実行できる。

---

## 2. マニフェストスキーマ

### ファイル配置

```
.github/gh-sync/
├── config.yaml          # 同期マニフェスト(本セクションで定義)
└── patches/            # patch 戦略で使用する unified diff ファイル群
    ├── Cargo.toml.patch
    └── ...
```

マニフェストファイル: `.github/gh-sync/config.yaml`(downstream/fork リポジトリ内)

#### YAML vs TOML

| 観点                  | YAML                               | TOML                                          |
| --------------------- | ---------------------------------- | --------------------------------------------- |
| Rust との親和性       | `serde_yml` クレートが必要         | `toml` クレートは Cargo.toml と共通           |
| コメント              | 対応                               | 対応                                          |
| 型の厳格さ            | 文字列/bool の混在ミスが起きやすい | 型が明示的で strict                           |
| 配列の記述            | `- key: val` で簡潔                | `[[rules]]` ブロックで冗長になりがち          |
| `.github/` との一貫性 | GitHub Actions 等と統一感あり      | Rust プロジェクトの `Cargo.toml` と統一感あり |
| エディタ補完          | yaml-language-server が充実        | taplo 等が対応                                |

**現時点の判断**: rules の配列が長くなる本マニフェストでは `[[rules]]` 記法の冗長さがネック。
**YAML を採用**する。`serde(deny_unknown_fields)` + JSON Schema で
TOML の strict 型チェックと同等の堅牢性を確保する。

### 2.1 トップレベル構造

```yaml
upstream:
  repo: <owner>/<name> # 必須
  ref: <branch|tag|sha> # 任意、デフォルト: "main"

rules:
  - <rule>
...
```

#### `upstream` オブジェクト

| フィールド | 型     | 必須   | デフォルト | 説明                                         |
| ---------- | ------ | ------ | ---------- | -------------------------------------------- |
| `repo`     | string | はい   | —          | `owner/name` 形式の GitHub リポジトリ        |
| `ref`      | string | いいえ | `"main"`   | 取得元の git ref(ブランチ、タグ、commit SHA) |

#### トップレベルの制約

- `upstream` は必須
- `rules` は必須、空でない配列
- 未知のトップレベルフィールドは不可(strict パース)

### 2.2 ルール構造

```yaml
- path: <string>
  strategy: <replace|create_only|delete|patch>
  patch: <string> # patch 戦略のみ必須
```

#### 共通フィールド(全戦略)

| フィールド | 型     | 必須 | 説明                                                   |
| ---------- | ------ | ---- | ------------------------------------------------------ |
| `path`     | string | はい | ローカル(downstream)側の相対パス。適用先となる         |
| `strategy` | enum   | はい | `replace`, `create_only`, `delete`, `patch` のいずれか |

`path` はローカルへの書き込み先を示す。upstream からの取得先は原則 `path` と同一だが、
`replace` / `create_only` では `source` で上書きできる。

#### `replace` / `create_only` の追加フィールド

| フィールド | 型     | 必須   | デフォルト    | 説明                                                                     |
| ---------- | ------ | ------ | ------------- | ------------------------------------------------------------------------ |
| `source`   | string | いいえ | `path` と同じ | upstream リポジトリ内の取得元パス。upstream 側だけパスが異なる場合に指定 |

例: upstream では `templates/ci.yaml` に置かれているが、
ローカルには `.github/workflows/ci.yaml` として配置したい場合:

```yaml
- path: .github/workflows/ci.yaml
  strategy: replace
  source: templates/ci.yaml
```

`delete` は upstream を参照しないため `source` は不要。
`patch` は patch ファイルが `path`(= upstream 取得先)に対して生成されるため、
`source` ≠ `path` になると patch が成立しない — `source` は使用不可。

#### `patch` 戦略の追加フィールド

| フィールド | 型     | 必須   | デフォルト                             | 説明                                                    |
| ---------- | ------ | ------ | -------------------------------------- | ------------------------------------------------------- |
| `patch`    | string | いいえ | `.github/gh-sync/patches/<path>.patch` | unified diff ファイルのパス(リポジトリルートからの相対) |

省略時は `path` をそのまま使い慣例パスを自動解決する。
慣例から外れる配置にしたい場合のみ明示指定する:

```yaml
# 省略形(慣例パスを自動使用)
- path: Cargo.toml
  strategy: patch

# 明示指定(慣例から外れる場合のみ)
- path: Cargo.toml
  strategy: patch
  patch: somewhere/Cargo.toml.patch
```

`patch` 戦略では `path` が upstream の取得先とローカルの適用先の両方を兼ねる。
取得先と適用先が一致しないと patch ファイルの文脈が崩れるため、`source` は指定不可。

#### 組み合わせ禁止ルール

- `delete`: `source`, `patch` フィールド不可
- `patch`: `source` フィールド不可
- `replace`, `create_only`: `patch` フィールド不可
- いずれのルールでも未知のフィールドはバリデーションエラー

### 2.3 パスの制約

`path` と `source` の両方に適用:

- 空文字列不可
- `/` 始まり不可(相対パス必須)
- `..` セグメント不可(パストラバーサル防止)
- `\` 不可(全プラットフォームで `/` を使用)
- 正規化済みであること(`./` プレフィックス不可、末尾 `/` 不可、二重 `//` 不可)

`path` のみ:

- 全ルールで一意であること

### 2.4 バリデーションレベル

バリデーションは2段階で実行する:

#### Stage 1: スキーマバリデーション(オフライン、ネットワーク不要)

upstream を取得せずに実行できるチェック:

1. YAML 構文が有効
2. 全必須フィールドが存在
3. `upstream.repo` が `^[a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+$` にマッチ
4. `strategy` が4つの許可値のいずれか
5. 戦略固有の必須フィールドが存在
6. 組み合わせ禁止ルールに違反しない(§2.2 参照)
7. 未知のフィールドがない
8. `path` および `source`(指定時)がパス制約を満たす(§2.3 参照)
9. `path` の重複がない

#### Stage 2: 参照バリデーション(ローカルファイルシステム)

ローカルファイルの読み取りが必要なチェック:

1. `patch` 戦略: `patch` フィールド指定時はそのパスに、省略時は慣例パス(`.github/gh-sync/patches/<path>.patch`)に patch ファイルが存在する

注意: upstream ファイルの存在はバリデーション時にはチェックしない。
ネットワークアクセスが必要なため、sync/ci-check 実行時のランタイムエラーとする。

### 2.5 例

各戦略を1つずつ示す。

```yaml
upstream:
  repo: naa0yama/boilerplate-rust
  ref: main

rules:
  # replace: upstream と同一内容でローカルを上書き
  - path: .github/workflows/rust-ci.yaml
    strategy: replace

  # replace + source: upstream 側だけパスが異なる場合
  - path: .github/workflows/ci.yaml
    strategy: replace
    source: templates/ci.yaml

  # create_only: 存在しない場合のみ作成(以後プロジェクト側で管理)
  - path: .github/gh-sync/config.yaml
    strategy: create_only

  # delete: テンプレートで廃止されたレガシーファイルを削除
  - path: .github/labels.json
    strategy: delete

  # patch: patch フィールド省略 → .github/gh-sync/patches/Cargo.toml.patch を自動使用
  - path: Cargo.toml
    strategy: patch

  # patch (ネストしたパス): 同様に省略 → .github/gh-sync/patches/.github/workflows/ci.yaml.patch
  - path: .github/workflows/ci.yaml
    strategy: patch
```

### 2.6 serde デシリアライゼーション

- `#[serde(deny_unknown_fields)]` で全レベルの未知フィールドを拒否
- `upstream.ref` は `#[serde(default)]` でデフォルト値 `"main"`
- `strategy` は小文字 enum (`#[serde(rename_all = "snake_case")]`)
- `patch` フィールドは `Option<String>` — 省略時は慣例パスを使用(§2.2 参照)

### 2.7 将来の検討事項

- エディタ補完用の JSON Schema 生成(yaml-language-server 対応)
- ルール単位の `source` フィールドオーバーライド(マルチソース同期)
- `path` での glob パターン対応(例: `.github/workflows/*.yaml`)

### 2.8 patch ファイルの管理方針

`patch` 戦略で使用する patch ファイルは `.github/gh-sync/patches/` 以下に、
対象ファイルのディレクトリ構造をそのまま維持して配置する:

```
.github/gh-sync/patches/
├── Cargo.toml.patch
└── .github/
    └── workflows/
        └── ci.yaml.patch
```

パスを加工(スラッシュをドット等に変換)しない理由:

- 変換ロジックが実装・テストのコストになる
- ディレクトリ構造が失われると、どのファイルの patch かが一目でわからない
- `path` に `.patch` を付加するだけで取得できる単純なルールを維持する

マニフェスト内の `patch` フィールドは明示指定のため、慣例はあくまで推奨:

```yaml
- path: .github/workflows/ci.yaml
  strategy: patch
  patch: .github/gh-sync/patches/.github/workflows/ci.yaml.patch
```

#### patch ファイルの再生成

patch ファイルは upstream が更新されるとコンフリクトする可能性がある。
再生成が必要なタイミング:

- プロジェクト新規セットアップ時(patch ファイルがまだ存在しない)
- `gh-sync sync` 実行時にコンフリクト warn が出た後
- upstream の変更を取り込む PR のレビュー前

`gh-sync sync --patch-refresh` を実行する(§4.4 参照)。
`strategy: patch` の全ルールに対して upstream との diff を `patches/` 以下に書き出すので、
内容を確認してコミットする。

---

## 3. 戦略

戦略は4種類: `replace`, `create_only`, `delete`, `patch`。
部分適用のニーズは `patch` 戦略で統一する。
patch ファイルは `gh-sync sync --patch-refresh` で自動生成できるため、
マニフェスト内に複雑なマーカー記法を持ち込む必要がない。

全戦略の中核は純関数として定義する:

```
fn apply(rule, upstream_content, local_content) -> Result<StrategyResult>
```

これによりファイルシステムやネットワークアクセスなしでのユニットテストが可能になる。

### 3.1 `replace`

upstream バージョンでローカルファイルを置換する。**ファイル単位のみ対応**。

```yaml
- path: .clippy.toml
  strategy: replace
```

アルゴリズム:

1. upstream ファイルの内容を取得
2. ローカルファイルを読み取り(存在する場合)
3. 比較: 同一なら `Unchanged` を返す
4. upstream の内容をローカルパスに書き込み
5. 必要に応じて親ディレクトリを作成
6. `Changed` を返す

ディレクトリ同期は対応しない。ディレクトリ以下の各ファイルをルールとして個別に列挙する。
(`gh api` のディレクトリレスポンス判定、ローカルとの型不一致処理など実装コストが高く、
どのファイルを管理対象とするかはマニフェストで明示すべきであるため)

#### upstream 404 の処理

upstream パスが 404 を返した場合: **warning** メッセージ付きで `Skipped` を返す。

```
upstream not found: {path} (use 'delete' strategy to remove local file)
```

ローカルファイルは変更も削除もしない。
これは gh-infra の障害モードを回避するための意図的な設計判断。gh-infra では 404 が
CLI 全体のエラー終了を引き起こし、後続ルールの実行を阻害していた。
削除は `delete` 戦略による明示的なアクションでなければならない。

#### エラー条件

- `gh` CLI エラー(404 以外) → エラー

### 3.2 `create_only`

ローカルパスが存在しない場合のみ upstream からコピーする。

```yaml
- path: .github/project-config.json
  strategy: create_only
```

アルゴリズム:

1. ローカルパスが存在するか確認
2. 存在する → `Skipped` を返す(理由: "already exists")
3. upstream ファイルの内容を取得
4. ローカルパスに書き込み(必要に応じて親ディレクトリを作成)
5. `Changed` を返す

ファイルのみ対応。ディレクトリパスで `create_only` を使用するとエラー
(曖昧なセマンティクス: 一部のファイルが存在し一部が存在しない場合の扱いが不明確)。

**`--ci-check` での drift 判定**: ファイルの **存在のみ** を確認する。
ファイルが存在すれば `OK`(中身が upstream と異なっていても drift としない)。
ファイルが存在しない場合は warning を出力して最終的に exit 1。

```
[WARN]   .github/gh-sync/config.yaml (create_only): file not found
```

`create_only` は「初回作成後はプロジェクト側で管理」する戦略のため、
内容の差分を drift とみなすと戦略の意図に反する。

#### エラー条件

- パスがディレクトリを指している → エラー
- upstream パスが存在しない → エラー

### 3.3 `delete`

ローカルファイルまたはディレクトリを明示的に削除する。upstream fetch は行わない。

```yaml
- path: .github/rulesets
  strategy: delete
- path: .github/labels.json
  strategy: delete
```

この戦略が存在する理由: `replace` は upstream が 404 を返した場合に意図的にローカル
ファイルを削除しない。削除は明示的な宣言アクションでなければならない。これにより
upstream の再編成や一時的な API 障害による意図しないデータ損失を防ぐ。

**gh-infra からの教訓**: gh-infra は削除に `source: /dev/null` と `reconcile: mirror` を
使用していたが、CLI が「ファイルが意図的に削除された」と「API エラー」を区別する必要があった。
正当に削除された upstream ファイルが 404 を返した際、CLI は削除を実行する代わりに
エラー終了した。`delete` を独立した戦略として分離することで、この曖昧さを完全に
排除する — upstream fetch を行わないため、誤処理される 404 が発生しない。

アルゴリズム:

1. ローカルパスが存在するか確認
2. 存在しない → `Skipped` を返す(理由: "not found")
3. ファイル → ファイルを削除
4. ディレクトリ → 再帰的に削除
5. `Changed` を返す

#### エラー条件

- アクセス権限エラー → エラー
- (他のエラーなし: パスが存在しない場合は `Skipped` であり、エラーではない)

### 3.4 `patch`

upstream ファイルを取得し、ローカルの unified diff パッチを適用してから結果を書き込む。

```yaml
# patch フィールド省略(推奨) — 慣例パスを自動解決
- path: Cargo.toml
  strategy: patch
```

#### アルゴリズム

1. `patch` フィールドが省略されている場合、慣例パス `.github/gh-sync/patches/<path>.patch` を使用
2. upstream ファイルの内容を取得
3. upstream の内容を一時ファイルに書き込み
4. 一時ファイルに `patch -p0 --no-backup-if-mismatch < patchfile` を実行
5. パッチ適用後の結果を読み取り
6. 現在のローカルファイルと比較
7. 同一 → `Unchanged` を返す
8. パッチ結果をローカルパスに書き込み
9. `Changed` を返す

#### パッチファイル形式

`diff -u` で生成される標準的な unified diff 形式:

```diff
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -1,6 +1,6 @@
 [workspace]
 resolver = "3"
 members = [
-    "crates/gh-sync",
+    "crates/dtvmgr",
 ]
```

#### パッチファイルの作成

patch ファイルは手書きせず `gh-sync sync --patch-refresh` で生成する(§4.4 参照)。
生成後に内容を確認してコミットする。

#### コンフリクト時の挙動

コンフリクト(パッチ適用失敗)は **エラーではなく warning** として扱う:

- `[WARN] Cargo.toml (patch): conflict detected — skipped` を出力
- `patch` コマンドの stderr をそのまま warning メッセージに含める
- ファイルへの書き込みは行わず `Skipped(conflict)` を返す
- **後続ルールの処理は継続する** — check/diff は最後まで実行する
- 全ルール完了後、コンフリクトが1件以上あれば exit 1

これにより `--dry-run` や `--ci-check` 実行時でも全ルールの状態を一括確認できる。
コンフリクトが発生した場合は §2.8 の手順で patch ファイルを再生成する。

#### エラー条件

- upstream パスが存在しない → エラー
- patch ファイルがローカルに存在しない → エラー(バリデーション Stage 2 で検出)
- `patch` コマンドが見つからない → エラー
- fuzz 付きでパッチ適用 → warning(エラーではない、適用は続行)

### 3.5 戦略の結果型

各戦略は以下のいずれかを返す:

| ステータス        | 意味                                                                             | 終了コードへの影響 |
| ----------------- | -------------------------------------------------------------------------------- | ------------------ |
| `Changed`         | ファイルが変更された(dry-run では変更される)                                     | —                  |
| `Unchanged`       | ファイルは既に期待状態と一致                                                     | —                  |
| `Skipped(reason)` | ルールが意図的に適用されなかった(例: `create_only` で既存ファイル、upstream 404) | —                  |
| `Conflict`        | patch 適用がコンフリクト — warning を出力、書き込みをスキップ、処理は継続        | exit 1             |
| `Error(reason)`   | ルールの適用に失敗                                                               | exit 1             |

### 3.6 戦略選択ガイド

| シナリオ                                                                  | 戦略          |
| ------------------------------------------------------------------------- | ------------- |
| CI ワークフロー、lint 設定など完全コピーが必要                            | `replace`     |
| 初回作成後はプロジェクト側でカスタマイズする設定                          | `create_only` |
| テンプレートで廃止されたレガシーファイル                                  | `delete`      |
| プロジェクト固有の変更が必要(workspace、パッケージ名、部分オーバーライド) | `patch`       |

---

## 4. 動作モード

### 4.1 sync(デフォルト)

```
gh-sync sync [-m <manifest>] [--dry-run]
```

ファイルをローカルに書き込む。commit / PR は行わない。
commit・PR のワークフローは skill 経由で Claude に委ねる。

1. マニフェストをパースしバリデーション
2. upstream ファイルを取得(`GITHUB_TOKEN` 環境変数を使用)
3. 各ルールの戦略を適用

`--dry-run` を付けるとファイルへの書き込みを行わず diff のみ出力する。

### 4.2 validate (`--validate`)

```
gh-sync sync --validate [-m <manifest>]
```

ファイルの取得や変更を行わずにマニフェストの正しさを検証。
成功で exit 0、エラーで exit 1。ルールごとのステータスを出力。

### 4.3 CI check (`--ci-check`)

```
gh-sync sync --ci-check [-m <manifest>]
```

drift detection: 現在のローカル状態を同期後の期待状態と比較。
乖離がなければ exit 0、乖離があれば exit 1。バリデーションを内包。

ファイルは一切変更しない。

**GitHub Actions 環境での追加動作** (`GITHUB_ACTIONS=true` のとき):

1. drift が検出されたファイルごとに workflow command でアノテーションを出力する:
   ```
   ::error file={path},title=gh-sync drift::{path} is out of sync with upstream
   ```
2. 実行が PR に紐づいている場合 (`GITHUB_REF` が `refs/pull/*/merge` のとき)、
   `gh pr comment` でドリフトサマリーをコメントとして投稿する:
   ```
   gh pr comment $PR_NUMBER --body "..."
   ```
   コメント本文には drift したファイルの一覧と各ファイルの unified diff を含める。
   PR 番号は `GITHUB_REF` から `refs/pull/<number>/merge` の形式で抽出する。

GHA 環境以外ではアノテーション・PR コメントの処理はスキップする。

### 4.4 patch-refresh (`--patch-refresh`)

```
gh-sync sync --patch-refresh [-m <manifest>]
```

`patch` 戦略のルールに対して upstream の最新内容とローカルファイルの diff を取り、
patch ファイルを自動生成(上書き)する。patch ファイルを手書きするコストを排除する。

パイプライン:

1. マニフェストをパースしバリデーション
2. `strategy: patch` のルールのみ対象
3. upstream ファイルを取得
4. ローカルファイルを読み取り(存在しない場合は空として扱う)
5. `diff -u <upstream> <local>` を実行
6. 差分がなければ `Unchanged`(patch ファイルは更新しない)
7. 解決済みパス(省略時は慣例パス)に unified diff を書き込み
8. 親ディレクトリが存在しない場合は作成

ローカルファイルが存在しない場合: `diff -u <upstream> /dev/null` に相当する差分を
patch ファイルとして生成する。`strategy: patch` を明示しているにもかかわらず
ローカルファイルがないのは patch ファイルの作成漏れを意味するため、
upstream との完全な差分を生成して確認・編集できる状態にする。

**upstream もローカルも変更しない**: patch ファイルのみ更新する。

典型的な利用タイミング:

- プロジェクト新規セットアップ時(patch ファイルをゼロから生成)
- `gh-sync sync` でコンフリクト warn が出た後
- upstream の変更を取り込む PR のレビュー前

---

## 5. CLI インターフェース

### 5.1 サブコマンド構造

```
gh-sync [command]

Commands:
  sync    upstream テンプレートリポジトリからファイルを同期
  init    テンプレート同期設定ファイルを初期化
```

### 5.2 `gh-sync sync` フラグ

| フラグ            | 短縮 | 型   | デフォルト                    | 説明                                           |
| ----------------- | ---- | ---- | ----------------------------- | ---------------------------------------------- |
| `--manifest`      | `-m` | Path | `.github/gh-sync/config.yaml` | マニフェストパス                               |
| `--dry-run`       | `-n` | bool | false                         | プレビューのみ(diff 出力、ファイル変更なし)    |
| `--validate`      |      | bool | false                         | マニフェストバリデーションのみ                 |
| `--ci-check`      |      | bool | false                         | drift detection                                |
| `--patch-refresh` |      | bool | false                         | patch ファイルを upstream との diff で自動生成 |

`gh` CLI が `GITHUB_TOKEN` 環境変数を自動的に使用する。
commit / PR 操作はこのコマンドでは行わない — skill 経由で Claude に委ねる。

### 5.3 フラグの競合ルール

- `--validate`, `--ci-check`, `--patch-refresh` は相互排他
- `--dry-run` は sync モード(デフォルト)のみ有効。他のモードでは無視する

### 5.4 `gh-sync init` フラグ

| フラグ            | 短縮 | 型   | デフォルト                    | 説明                                                       |
| ----------------- | ---- | ---- | ----------------------------- | ---------------------------------------------------------- |
| `--repo`          | `-r` | str  | —(TTY の場合はプロンプト)     | upstream リポジトリ(`owner/name` 形式)                     |
| `--ref`           |      | str  | `main`                        | 取得元の git ref                                           |
| `--output`        | `-o` | Path | `.github/gh-sync/config.yaml` | 出力先パス                                                 |
| `--from-upstream` |      | bool | false                         | upstream 自身の config をコピー(非対話モード可)            |
| `--select`        |      | bool | false                         | upstream のファイル一覧から対話的に選択                    |
| `--with-workflow` |      | bool | false                         | `.github/workflows/gh-sync.yaml` を同時生成 (詳細は §11.3) |
| `--force`         |      | bool | false                         | 既存ファイルを確認なしで上書き                             |

`--from-upstream` と `--select` は相互排他。どちらも指定しない場合、TTY であればモード選択プロンプトを表示する。

config ファイルと同ディレクトリに `schema.json` (JSON Schema) を生成し、
yaml-language-server 対応エディタで補完・バリデーションが有効になる。

config が不在の状態で `gh-sync sync` を実行した場合、エラーメッセージに
`gh-sync init` 実行のヒントを表示する。

---

## 6. upstream ファイルの取得

### 6.1 取得方法

`gh` CLI 経由で取得する。API を直接叩かない。

#### ファイル取得

```
gh api repos/{owner}/{repo}/contents/{path}?ref={ref} --jq '.content' | base64 -d
```

`gh` が `GITHUB_TOKEN` の認証・レート制限・エラーハンドリングを担う。

#### ディレクトリ一覧

```
gh api repos/{owner}/{repo}/contents/{path}?ref={ref} --jq '.[].path'
```

エントリを再帰的に取得し、各ファイルを上記のファイル取得コマンドで読み込む。

#### 404 判定

`gh api` は HTTP 404 時に非ゼロで終了する。終了コードで存在確認を行う。

### 6.2 レート制限

`gh` CLI が自動で処理する。個別の `X-RateLimit-*` ヘッダ管理は不要。

---

## 7. commit / PR ワークフロー

`gh-sync sync` は **ファイルの書き込みのみ** 行い、commit / PR 操作は行わない。
main ブランチはプロテクトされているため、変更は必ずブランチ → PR 経由でマージする。

commit・PR の作成は skill 経由で Claude に委ねる。
これにより commit メッセージ・署名・PR 本文の内容が正確になり、
`gh-sync sync` 側に git/gh の複雑な操作を持ち込まずに済む。

### 7.1 ローカル実行フロー

```
gh-sync sync              # ファイルをローカルに書き込む
↓
(変更内容を確認)
↓
/commit スキル          # ブランチ作成・commit・PR 作成を Claude が担当
```

### 7.2 CI フロー (`--ci-check`)

```yaml
- name: drift detection
  run: gh-sync sync --ci-check
  env:
    GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

乖離があれば exit 1 で CI を失敗させる。修正は別途ローカルで `gh-sync sync` を実行する。

---

## 8. 出力形式

### 8.1 sync / dry-run 出力

ルールごとのステータス行に加え、変更があったファイルにはファイル単位の unified diff を表示する。

```
[CHANGED]  .github/workflows/ci.yml (replace)
--- a/.github/workflows/ci.yml
+++ b/.github/workflows/ci.yml
@@ -10,3 +10,4 @@
     - uses: actions/checkout@v4
     - uses: actions/setup-node@v4
+    - uses: actions/cache@v4

[OK]       .clippy.toml (replace)
[SKIPPED]  .github/project-config.json (create_only): already exists
[CHANGED]  Cargo.toml (patch)
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -5,3 +5,3 @@
 members = [
-    "crates/gh-sync",
+    "crates/dtvmgr",
 ]

[DELETED]  .github/labels.json (delete)
---
2 changed, 1 deleted, 1 up to date, 1 skipped
```

`--dry-run` 時は diff のみ表示し、ファイルへの書き込みは行わない。
diff 形式は `diff -u` 互換(ファイルパスのプレフィックスは `a/`, `b/`)。

### 8.2 validate 出力

```
[OK]    YAML syntax valid
[OK]    upstream.repo: naa0yama/boilerplate-rust
[OK]    rule[0] .github/workflows/ci.yml: replace
[OK]    rule[1] Cargo.toml: patch -> .github/gh-sync/patches/Cargo.toml.patch (exists)
[FAIL]  rule[2] mise.toml: patch -> .github/gh-sync/patches/mise.toml.patch (not found)
---
2 rules OK, 1 error
```

### 8.3 CI check 出力

drift が検出されたファイルには diff を表示する:

```
[DRIFT]  .github/workflows/ci.yml (replace): upstream has changes
--- a/.github/workflows/ci.yml (local)
+++ b/.github/workflows/ci.yml (expected)
@@ -10,3 +10,4 @@
     - uses: actions/checkout@v4
     - uses: actions/setup-node@v4
+    - uses: actions/cache@v4

[OK]     .clippy.toml (replace): up to date
[DRIFT]  Cargo.toml (patch): patched result differs from local
--- a/Cargo.toml (local)
+++ b/Cargo.toml (expected)
@@ -5,3 +5,3 @@
 members = [
-    "crates/gh-sync",
+    "crates/dtvmgr",
 ]

---
2 files drifted, 1 up to date
```

**GHA 環境での追加出力** (`GITHUB_ACTIONS=true`):

アノテーション (stdout に出力、GHA がファイル位置表示に使用):

```
::error file=.github/workflows/ci.yml,title=gh-sync drift::.github/workflows/ci.yml is out of sync with upstream
::error file=Cargo.toml,title=gh-sync drift::Cargo.toml is out of sync with upstream
```

PR コメント本文 (PR に紐づく実行のみ `gh pr comment` で投稿):

````
## gh-sync drift detected

| file | strategy | status |
| ---- | -------- | ------ |
| `.github/workflows/ci.yml` | replace | DRIFT |
| `Cargo.toml` | patch | DRIFT |

<details>
<summary>.github/workflows/ci.yml diff</summary>

```diff
@@ -10,3 +10,4 @@
     - uses: actions/checkout@v4
     - uses: actions/setup-node@v4
+    - uses: actions/cache@v4
````

</details>

Run `gh-sync sync` locally to apply upstream changes.

---

## 9. エラーハンドリング

| 種別                     | 挙動                                                                                | 終了コード     |
| ------------------------ | ----------------------------------------------------------------------------------- | -------------- |
| マニフェストパースエラー | 行/列を可能な限り報告して即時終了                                                   | exit 1         |
| upstream 取得エラー      | `gh` CLI の終了コード・stderr とパスを報告して即時終了                              | exit 1         |
| upstream 404             | warning を出力して `Skipped` — 後続ルールへ継続                                     | —              |
| patch コンフリクト       | warning + `patch` stderr を出力して `Conflict` — 後続ルールへ継続、書き込みはしない | exit 1(最後に) |
| 戦略エラー               | ルールインデックス・パス・戦略・理由を報告 — 後続ルールへ継続                       | exit 1(最後に) |
| Git エラー               | stderr を報告して即時終了                                                           | exit 1         |

**全ルール処理後に終了コードを確定する**: `Conflict` または `Error` が1件以上あれば exit 1。
途中でも即時終了する条件はマニフェストパースエラー・upstream 取得エラー・Git エラーのみ。

---

## 10. 設計決定ログ

以下の判断はすべて確定済み。

1. **`replace` のディレクトリ処理**: ファイル単位のみ。ディレクトリ対応なし。
   `gh api` のレスポンス型判定やローカル型不一致処理など実装コスト過大。管理ファイルはマニフェストで個別に列挙すべき。

2. **レート制限処理**: `gh` CLI に委任。
   `gh` がリトライ・バックオフを自動処理するため、アプリ側での実装は不要。

3. **PR コメントテンプレート**: 不要。
   `gh-sync sync` は PR 作成を担わない。ローカル実行は skill 経由で Claude が対処。GHA `--ci-check` の PR コメントはハードコードで十分。

4. **`--ci-check` での `create_only` drift 判定**: ファイルの存在のみ確認。
   存在すれば `OK`。存在しなければ `warn + exit 1`。中身の差分は drift とみなさない(「初回作成後はプロジェクト側で管理」という戦略の意図)。

5. **`--patch-refresh` でローカルファイル不在**: upstream との完全差分を patch として生成。
   `strategy: patch` を明示しているのに patch ファイルがないのは作成漏れ。生成して確認・編集できる状態にするのが親切。

---

## 11. GitHub Action

### 11.1 使い方

`action.yml` をリポジトリ直下に配置することで、下流リポジトリから
`uses: naa0yama/gh-sync@<tag>` の形式で呼び出せる公開 Action として機能する。

実行シーケンス (固定・書き込みなし):

1. `gh-sync sync file --validate` — マニフェストのスキーマ検証 (ローカル、upstream 接続なし)
2. `gh-sync sync repo --ci-check` — リポジトリ設定のドリフト検知
3. `gh-sync sync file --ci-check` — ファイルのドリフト検知

いずれかのステップが失敗すると Action は即時 exit 1 する (GitHub Actions のデフォルト fail-fast)。
ドリフトを **取り込む** 責務は持たず、検知のみ。

### 11.2 inputs

| name                | required | default                       | 説明                                                                       |
| ------------------- | :------: | ----------------------------- | -------------------------------------------------------------------------- |
| `token`             |   yes    | —                             | `gh release download` と `gh` CLI 内部で使用するトークン                   |
| `version`           |    no    | `github.action_ref`           | ダウンロードするリリースタグ (例: `v0.1.3`)。SHA pin 時は明示必須          |
| `manifest`          |    no    | `.github/gh-sync/config.yaml` | 同期設定ファイルのパス                                                     |
| `upstream-manifest` |    no    | —                             | upstream マニフェスト参照 (`owner/repo@ref:path` 形式)。詳細は第 12 章参照 |

### 11.3 gh-sync init --with-workflow

`gh-sync init` に `--with-workflow` フラグを追加。config.yaml・schema.json の生成に加えて
`.github/workflows/gh-sync.yaml` を生成する。

- 非インタラクティブ (`stdin` が TTY でない) 時は `--with-workflow` を明示した場合のみ生成。
- TTY 時はモード選択後にインタラクティブな確認プロンプトを表示する。
- 既存ファイルがある場合は `--force` がなければ上書き確認を行う (非 TTY では bail)。
- 埋め込まれるバージョンは `gh-sync` 実行時の `CARGO_PKG_VERSION` (例: `v0.1.3`)。

```bash
# 非インタラクティブ例
gh-sync init --repo naa0yama/boilerplate-rust --from-upstream --with-workflow --force
```

生成される `.github/workflows/gh-sync.yaml` の内容 (`--repo owner/repo` 指定時):

```yaml
# yaml-language-server: $schema=https://json.schemastore.org/github-workflow.json
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
      - uses: naa0yama/gh-sync@<version>
        with:
          token: ${{ secrets.GITHUB_TOKEN }}
          upstream-manifest: owner/repo@main:.github/gh-sync/config.yaml
```

## 12. upstream manifest と local overlay

### 12.1 概要

マニフェストを各 downstream リポジトリに個別に配置するのではなく、
upstream テンプレートリポジトリ (例: `naa0yama/boilerplate-rust`) で一元管理し、
downstream は `--upstream-manifest` でそれを参照する構成を取れる。

downstream が特殊なカスタマイズを必要とする場合は、ローカルに overlay マニフェストを
置くことで upstream の定義を選択的に上書き・除外できる。

### 12.2 upstream manifest 参照形式

```
owner/repo@ref:path
```

| 要素         | 説明                                        | 省略時のデフォルト |
| ------------ | ------------------------------------------- | ------------------ |
| `owner/repo` | upstream リポジトリ (`owner/name` 形式必須) | —                  |
| `@ref`       | ブランチ、タグ、またはコミット SHA          | `HEAD`             |
| `:path`      | リポジトリ内のマニフェストファイルパス      | (省略不可)         |

**例:**

```
naa0yama/boilerplate-rust@main:.github/gh-sync/config.yaml
naa0yama/boilerplate-rust@v1.0.0:.github/gh-sync/config.yaml
naa0yama/boilerplate-rust:.github/gh-sync/config.yaml  # ref 省略 → HEAD
```

### 12.3 マニフェスト解決ロジック

1. `--upstream-manifest` が指定された場合:
   - 指定された参照を `gh api` で取得し、YAML をパースする
   - `--manifest` に指定されたローカルファイルが存在すれば、`merge_overlay` を適用する
   - ローカルファイルが存在しない場合は upstream マニフェストをそのまま使用する
2. `--upstream-manifest` が指定されない場合:
   - 従来どおり `--manifest` のローカルファイルのみを使用する

### 12.4 マージ規則

`merge_overlay(upstream, local)` の適用規則:

#### `upstream:` ノード

local マニフェストが存在する場合、local の `upstream:` ノードが upstream のものを
**完全に置換**する。部分マージは行わない。

#### `spec:` ノード

フィールド単位で **local 優先**マージを行う。`Option<T>` フィールドは
local が `Some` のときのみ上書きし、`None` のときは upstream の値を継承する。

#### `files:` リスト

`path` を key にマージする:

| 状態                              | 結果                                |
| --------------------------------- | ----------------------------------- |
| upstream のみに存在               | upstream ルールをそのまま採用       |
| local のみに存在                  | local ルールを末尾に追加            |
| 両方に存在                        | local ルールで **完全置換**         |
| 両方に存在 かつ local が `ignore` | 当該 path をマージ結果から **削除** |
| local のみに存在 かつ `ignore`    | マージ結果に追加しない (無視)       |

### 12.5 strategy: ignore

`strategy: ignore` は **local overlay 専用**の戦略で、upstream で定義されたルールを
downstream が明示的に除外するために使う。

```yaml
# local overlay (.github/gh-sync/config.yaml)
upstream:
  repo: naa0yama/boilerplate-rust
  ref: main
files:
  # upstream で定義された Cargo.toml の patch ルールを除外する
  - path: Cargo.toml
    strategy: ignore
  # upstream にないローカル固有のルールを追加
  - path: .project-config.json
    strategy: create_only
```

**制約:**

- `source` フィールドは指定できない (`delete` と同じ制約)
- `patch` フィールドは指定できない
- ドリフト検知・sync の両対象から除外される (patch ファイルの存在チェックも対象外)

### 12.6 GitHub Actions での使用例

**純 upstream 動作** (downstream に local manifest なし):

```yaml
- uses: naa0yama/gh-sync@v0.1.3
  with:
    token: ${{ secrets.GITHUB_TOKEN }}
    upstream-manifest: naa0yama/boilerplate-rust@main:.github/gh-sync/config.yaml
```

**local overlay あり** (downstream に上書きルールを定義):

```yaml
- uses: naa0yama/gh-sync@v0.1.3
  with:
    token: ${{ secrets.GITHUB_TOKEN }}
    upstream-manifest: naa0yama/boilerplate-rust@main:.github/gh-sync/config.yaml
    manifest: .github/gh-sync/config.yaml
```
