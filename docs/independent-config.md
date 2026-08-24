# 单一配置约定

## 目标

`mihomo-tui` 只拥有一个配置文件：

```text
/etc/mihomo-tui/config.yaml
```

该文件是原生 Mihomo YAML。`mihomo-tui` 与受管 Mihomo 服务读取同一个文件，不生成运行时副本，也不存在第二个由 `mihomo-tui` 管理的配置档案。

Mihomo 仍是可选的运行时后端。即使 Mihomo 已停止、尚未安装或无法连接，TUI 也能打开和编辑该文件。

## 文件所有权

- 日常读取和编辑都使用 `/etc/mihomo-tui/config.yaml`。
- 订阅、规则、TUN 和 DNS 变更直接写入该文件。
- 源文件备份创建在同一目录，权限为 `0600`。
- 受管 Mihomo 服务使用 `/etc/mihomo-tui` 作为数据目录，因此会直接读取同一个 `config.yaml`。
- `/etc/mihomo/config.yaml` 不会被同步、生成或更新。

每次编辑都必须保留未知字段和用户自定义的 Mihomo 字段。

## 首次运行迁移

当 `/etc/mihomo-tui/config.yaml` 不存在时，初始化按以下顺序执行：

1. 如果显式指定了旧配置，导入该文件。
2. 否则，如果 `/etc/mihomo/config.yaml` 存在，导入该文件。
3. 否则，创建最小的原生 Mihomo 配置。

导入会复制完整 YAML 映射，不修改或删除旧文件。新文件一旦存在，初始化就不会再次导入。

早期版本使用过以下包装格式：

```yaml
kind: mihomo-tui/v1
backend: mihomo
profile:
  # Mihomo YAML 配置
```

初始化会自动用完整的 `profile` 映射替换该包装，并保留私有备份。此格式迁移只执行一次。

配置文件以原子方式创建，权限为 `0600`。YAML 格式错误、根节点不是映射、不支持的包装格式、符号链接源文件或符号链接配置目录都会中止初始化，且不会留下不完整文件。

## 离线编辑

每项编辑操作都会解析原生 YAML，只修改自身负责的字段，保留未受管字段，再以原子方式替换同一个配置文件。旧版本会作为权限为 `0600` 的备份保留。

编辑不会安装或启动 Mihomo，也不会调用 Mihomo 可执行文件。界面会报告文件已保存；如果使用本机托管后端，还会提示运行中的内核是否仍需重新加载配置。

## 应用与重载

按 `p` 是运行时操作，不是文件同步操作。本机托管后端会执行：

1. 准备或按需安装 Mihomo，但不会让一个原本停止的服务带着其他配置意外启动。
2. 要求现有内核位于 `managed-core.json` 内嵌的兼容范围。全新主机安装必须与清单中的推荐版本及包元数据完全一致。
3. 使用受信任的 Mihomo 可执行文件校验 `/etc/mihomo-tui/config.yaml`。
4. 安装 root 所有的 systemd drop-in，把 Mihomo 数据目录设为 `/etc/mihomo-tui`。
5. 重新加载 systemd，并执行 `systemctl restart mihomo.service`。
6. 刷新已配置的 HTTP provider，并验证其能够返回节点。

校验失败时，已保存的配置仍可继续修正，服务不会重启。受管 drop-in 是唯一支持的覆盖配置；程序会拒绝未知的服务 drop-in，而不是覆盖它们。

启动和应用都不会升级现有内核。新的 Mihomo 上游版本必须先加入经过审查的清单，并通过 `docs/managed-core.md` 说明的托管内核发布检查。

外部 Controller 模式或显式旧配置模式绝不管理本机 systemd。配置仍可编辑，但本机应用功能不可用。

## 命令行约定

- `--workspace` / `MIHOMO_TUI_CONFIG` 选择唯一的受管配置，默认值为 `/etc/mihomo-tui/config.yaml`。显式自定义路径会进入外部模式，绝不修改本机 systemd。
- `--config` / `MIHOMO_CONFIG` 选择只在首次导入时使用的旧配置。提供该参数会进入外部模式，绝不管理本机 systemd。
- `--controller` / `MIHOMO_CONTROLLER` 与 `--secret` / `MIHOMO_SECRET` 覆盖 Controller 发现结果，但不修改配置文件。
- `--no-auto-install` 阻止按 `p` 时安装 Mihomo，不影响启动或离线编辑。

## 验收条件

- Mihomo 已停止、缺失或不可连接时，TUI 仍能打开和保存配置。
- 添加或替换订阅会直接修改 `/etc/mihomo-tui/config.yaml`。
- 任何编辑或应用操作都不会写入 `/etc/mihomo/config.yaml`。
- 首次导入保留未知的嵌套字段，并保证被导入文件逐字节不变。
- 现有包装格式工作区迁移时不丢失任何 `profile` 字段。
- 单一配置及其备份在 Unix 上的权限均为 `0600`。
- 受管 Mihomo 以 `/etc/mihomo-tui` 为数据目录启动和重载。
- 只有显式应用时才执行校验，校验不能阻止离线编辑。
- 界面不显示订阅凭据或 API 密钥。
