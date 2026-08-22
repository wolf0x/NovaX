---
name: whoamifuck
description: Linux intrusion detection & incident-response reporting tool（Whoamifuck / 司稽）。可执行主机安全巡检：设备信息、登录/爆破日志分析、进程与服务、开放端口+HTTP 探测、计划任务/历史命令/异常文件、用户账户、Webshell 查杀、已知 CVE 检测（redis、OpenSSH、sudo、XZ、Dirty COW/Pipe）、安全基线、Rootkit 查杀（chkrootkit/rkhunter），并可导出文本/HTML 报告。必须以 root 运行。
triggers:
  - whoamifuck
  - 司稽
  - incident response
  - intrusion detection
  - 入侵检测
  - 应急响应
  - webshell 查杀
  - rootkit
  - 安全基线
  - 攻击溯源
---

# Whoamifuck（司稽）— Linux 入侵检测与应急响应报告工具

脚本位于本技能目录 `scripts/who.sh`（v7.1.0，作者 Enomothem）。**必须以 root 运行**：

```bash
sudo bash scripts/who.sh -h        # 帮助
sudo bash scripts/who.sh -a        # 全量快速巡检
```

纯 Bash 脚本、无构建步骤，保持可执行即可。

## CLI 参数速查（全部 19 项）

| 参数 | 长选项 | 对应模块 | 用途 |
|------|--------|----------|------|
| `-v` | `--version` | — | 版本信息 |
| `-h` | `--help` | — | 帮助 |
| `-u` | `--user-device` | `fk_baseinfo` | 设备基本信息：IP/掩码/网关/DNS/主机名/系统/内核、在线用户、last/lastlog、计划任务计数 |
| `-l FILE` | `--login FILE` | `fk_userlogin` | 登录日志分析：会话开启/关闭、爆破攻击 IP、成功登录（按发行版自动选择 auth.log / secure） |
| `-n` | `--nomal` | 普通模式 | 基本信息 + 历史命令 + 计划任务 + 文件变更 + 文件列表 + 用户信息 |
| `-a` | `--all` | 全量模式 | 基本信息 + 系统状态 + 登录日志 |
| `-x` | `--proc-serv` | `fk_procserv` | 进程列表 + 运行中的服务（systemd） |
| `-p` | `--port` | `fk_portstatus`/`port_http` | 开放端口 + curl 探测 HTTP 标题/状态码 |
| `-s` | `--os-status` | `fk_devicestatus` | 内存 / 磁盘 / CPU 负载状态 |
| `-b` | `--baseline` | `fk_baseline` | 安全基线：身份鉴别、访问控制、安全审计、资源控制（含预期结果与整改建议） |
| `-r` | `--risk` | `fk_vulcheck` | CVE 检测：redis 未授权/弱口令、OpenSSH CVE-2018-15473 / CVE-2024-6387、sudo CVE-2019-18634 / CVE-2021-3156 / CVE-2023-22809、XZ CVE-2024-3094、Dirty COW、Dirty Pipe |
| `-k` | `--rootkitcheck` | `fk_rookit_analysis` | 运行 chkrootkit + rkhunter，结果保存到 `output/` |
| `-w PATH` | `--webshell PATH` | `fk_wsfinder` | Webshell 查杀（PHP/JSP 规则），默认 web 目录 `/www/wwwroot`、`/var/www`，输出 `output/webshell.txt` |
| `-c URL|FILE` | `--code URL|FILE` | HTTP 存活探测（单 URL 或列表文件）→ `output/http_info.txt` |
| `-i FILE` | `--sqletlog FILE` | `fk_weblog_sqlianalysis` | Web 访问日志 SQL 注入分析（盲注 `>`/`!=`、时间盲注） |
| `-e H M` | `--auto-run H M` | `fk_auto_run` | **修改 crontab** — 每日定时执行 `-m`；`-e c` 清除计划 |
| `-z PATH` | `--ext PATH` | `fk_extention` | **执行自定义命令**（来自 `~/.whok/chief-inspector.conf`，`commands=(...)`，`;` 分隔 命令;描述）。缺失时生成模板 |
| `-t on|off` | `--terminalproxy` | `fk_terminal_proxy` | 切换 Clash 代理环境（写/加载 `~/.clash`，访问 cip.cc / google.com） |
| `-y` | `--whoamifuck` | `fk_autofuck` | 彩蛋：溯源思路笔记 |
| `-o FILE` | `--output FILE` | `fk_output` | 导出文本报告 + hash/日志 → `output/text/report-<时间戳>.tar.gz` |
| `-m FILE` | `--html FILE` | `fk_reporthtml` | 导出 HTML 报告（带进度条）→ `output/html/` |

## 推荐应急响应流程

1. **快速研判** — `-u`（设备与用户）+ `-l`（登录/爆破证据）。
2. **持久化排查** — `-n` 普通模式（历史命令、计划任务、文件变更、账户），按需加 `-x`、`-p`。
3. **恶意文件排查** — `-w`（webshell）、`-k`（rootkit，需已装 chkrootkit/rkhunter）。
4. **暴露面评估** — `-r`（已知 CVE）与 `-b`（安全基线）。
5. **取证留档** — `-m 报告名`（HTML）或 `-o 名称`（文本 tar.gz），再从 `output/` 收集。

## 安全注意事项

- **会改变系统状态 / 高风险参数：** `-e`（写 crontab）、`-z`（执行配置中的任意命令）、`-t`（写代理环境、访问外部站点）。除非用户明确要求，否则**不要运行**；执行 `-z` 前先审查 `~/.whok/chief-inspector.conf` 内容。
- 仅以 root 运行；部分模块内部还会调用 `sudo`（chkrootkit、rkhunter、lsof）。非交互场景需确保免密 sudo 或整体以 root 运行。
- 模块会在当前目录 `output/` 写产物、在 `/tmp` 写临时文件——请从独立工作目录运行，便于收集报告。
- 输出为中文彩色文本；请为用户归纳结论并翻译关键证据。
- 已知脚本缺陷（不要静默修复）：`user_centos_defi` 赋值有误且未绑定任何参数；`fk_terminal_proxy` 会访问 cip.cc/google.com；`fk_http_scan`/`port_http` 会追加写入 `output/http_info.txt`。

## 输出处理

用 `sys_read` 读取 `output/` 目录下的报告产物并汇总回复。保留 `output/webshell.txt`、`output/vuln.txt`、`output/http_info.txt` 及生成的报告作为证据。
