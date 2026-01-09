# joinbell

<img alt="joinbell screenshot" src="https://github.com/user-attachments/assets/79e828e5-3f99-4dd5-a513-2e1bfb33068a" />

## 概要

joinbell は、Discord 上でゲーム募集を簡単に行うための Bot です。
募集メッセージへのリアクションを参加意思として扱い、一定人数で開始通知を送ります。
詳細な仕様は [spec.md](./spec.md) を参照してください。

## 起動

1. 環境変数または`.env`で`DISCORD_TOKEN` を設定します。
2. 以下を実行します。

```sh
cargo run
```

## 使い方

### 募集メッセージ作成

- スラッシュコマンド `/recruit` を実行して募集を作成します。
- 必須パラメータ
  - `game_title`: ゲームタイトル
  - `required_players`: 開始するのに必要な人数 (1 <= `required_players`)
- オプショナルパラメータ
  - `mention_role`: 開始通知でメンションするロール (指定しなければメンションしません)
  - `create_role`: `mention_role` が未指定のときにロールを作成するかどうか (既定: false)
  - `auto_assign_role_on_reaction`: リアクション時にロールを自動付与するかどうか (既定: `create_role` に連動し、`mention_role` がある場合のみ有効)
  - `notify_on_reaction`: 参加通知を送るかどうか (既定: true)
  - `delete_after_minutes`: 参加通知と開始通知を削除するまでの分数 (1 <= `delete_after_minutes`、既定: 60)

例:

```text
/recruit game_title:minecraft required_players:3 create_role:True auto_assign_role_on_reaction:True delete_after_minutes:30
```

### 参加

暇な時などにリアクションをつけると参加できます。

3人集まるとメンションが送られゲームを開始する合図になります。

:bell: を押すと今参加している人だけで開始することができます。

## 権限

- メッセージの送信
- メッセージの管理
- リアクションの追加
- ロールへのメンション
- ロール管理 (`create_role`, `auto_assign_role_on_reaction` を使う場合)
