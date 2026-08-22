---
name: security-audit
description: Linux 入侵检测与安全审计：涵盖用户登录分析、异常进程/端口/文件排查、SSH 后门检测、Webshell 扫描、已知漏洞检查、安全基线评估，输出结构化 Markdown 报告
---

# Linux 入侵检测与安全审计技能

> 灵感来源：[Whoamifuck](https://github.com/enomothem/Whoamifuck) — 精简提炼为 Agent 可执行的分步检查流程。

当用户要求"安全审计"、"入侵检测"、"安全检查"、"whoamifuck"或类似需求时，按以下模块依次执行。
每个模块使用 `sys_execute` 采集数据，最终汇总为结构化 Markdown 报告写入 `output/` 目录。

**前置要求**：需要 `execute` 门禁开启；部分检查（shadow 文件、chkrootkit 等）需要 root 权限。

---

## 模块 1：系统基本信息

```bash
# 网络
hostname
ip -4 addr show | grep -E 'inet |^[0-9]' | head -20
ip route | head -3
cat /etc/resolv.conf 2>/dev/null | grep nameserver

# 系统
uname -a
cat /etc/os-release | grep -E '^(NAME|VERSION_ID|PRETTY_NAME)='
uptime
who
last -i | head -10
lastlog | grep -v Never | head -20
```

**输出要点**：IP/掩码/网关、主机名、OS 版本、内核、运行时长、在线用户、最近登录记录。

---

## 模块 2：系统资源状态

```bash
free -m | awk 'NR==2{printf "内存使用: %.2f%%\n", $3*100/$2}'
df -h | awk '$NF=="/"{printf "磁盘使用: %s\n", $5}'
top -bn1 | head -5
```

**异常阈值**：内存 >85%、磁盘 >85% 需加粗告警。

---

## 模块 3：用户登录日志分析

自动检测发行版选择日志文件：
- Debian/Ubuntu/Kali → `/var/log/auth.log`
- CentOS/RHEL → `/var/log/secure`

```bash
# 判断发行版
grep PRETTY_NAME /etc/os-release

# 用户登录/登出（取最近 20 条）
grep "session opened" <LOG_FILE> | tail -20
grep "session closed" <LOG_FILE> | tail -20

# 暴力破解分析：攻击者 IP → 枚举用户名
grep "Failed password for invalid user" <LOG_FILE> \
  | awk '{print $13 " --> " $11}' | sort | uniq -c | sort -rn | head -20

# 攻击者 IP 排行
grep "Failed password for invalid user" <LOG_FILE> \
  | awk '{print $11}' | sort | uniq -c | sort -rn | head -20

# 登录成功的 IP
grep "Accepted" <LOG_FILE> | awk '{print $1,$2,$3, $11, $9}'

# 对已知用户名的爆破次数
grep "Failed password for" <LOG_FILE> | grep -v invalid \
  | awk '{print $9, $11}' | sort | uniq -c | sort -rn | head -20
```

**输出要点**：按表格呈现——攻击源 IP、尝试次数、目标用户名、时间范围。

---

## 模块 4：进程与服务排查

```bash
# 全部进程
ps aux --sort=-%mem | head -30

# 运行中的服务
systemctl list-units --type=service --state=running --no-pager

# 可疑进程检查：高 CPU/内存占用
ps aux --sort=-%cpu | awk 'NR<=10{print}'
```

**关注点**：未知进程名、伪装系统进程名的可疑进程、异常高资源占用。

---

## 模块 5：端口与网络

```bash
# 监听端口及关联进程
ss -tulpn

# HTTP 服务端口探测（排除已知非 HTTP 端口）
for port in $(ss -tuln | awk '/LISTEN/ && /tcp/ {split($5,a,":"); print a[length(a)]}' | sort -un); do
  case $port in 22|25|139|445|465|587|993|995|3306|3389) continue;; esac
  code=$(curl -s -o /dev/null -w "%{http_code}" --connect-timeout 3 "http://localhost:$port" 2>/dev/null)
  [ "$code" != "000" ] && echo "端口 $port → HTTP $code"
done
```

---

## 模块 6：历史命令与计划任务

```bash
# 用户历史命令
cat ~/.*sh_history 2>/dev/null | tail -30

# 计划任务排查
crontab -l 2>/dev/null
cat /etc/crontab 2>/dev/null
ls -la /var/spool/cron/ 2>/dev/null
```

**关注点**：可疑的定时下载/反弹 shell/加密货币挖矿命令。

---

## 模块 7：异常文件排查

```bash
# 近 3 天修改的文件
find / -type f -mtime -3 -not -path '/proc/*' -not -path '/sys/*' 2>/dev/null | head -50

# 近 3 天创建的文件
find / -type f -ctime -3 -not -path '/proc/*' -not -path '/sys/*' 2>/dev/null | head -50

# /home /opt 下最近修改的文件
for d in /home /opt; do
  [ -d "$d" ] && find "$d" -type f -mtime -3 2>/dev/null | head -20
done

# SSH authorized_keys 检查
for f in /home/*/.ssh/authorized_keys /root/.ssh/authorized_keys; do
  [ -f "$f" ] && echo "=== $f ===" && stat -c '%a %y %n' "$f" && cat "$f"
done
```

**关注点**：未知公钥写入 authorized_keys、SUID/SGID 文件、/tmp 下的可执行文件。

---

## 模块 8：用户信息排查

```bash
# 最近用户变更
tail -10 /etc/passwd
tail -10 /etc/shadow 2>/dev/null

# root 权限用户（UID=0）
awk -F: '$3==0{print $1}' /etc/passwd

# 可远程登录的用户（shadow 中密码非锁定）
awk -F: '$2!~/^[!*]/{print $1}' /etc/shadow 2>/dev/null

# sudo 权限用户
grep -v '^#\|^$' /etc/sudoers 2>/dev/null | grep 'ALL=(ALL)'

# 重复 UID 检查
awk -F: '{print $1, $3}' /etc/passwd | sort -k2 -n | uniq -D -f1
```

**告警**：多个 UID=0 用户、未知用户出现在 shadow 中。

---

## 模块 9：SSH 后门检测

```bash
# 检查是否有进程的 exe 软链指向 sshd（但本身不是真正的 sshd 端口）
for pid in $(ls /proc/ | grep -E '^[0-9]+$'); do
  [ -L "/proc/$pid/exe" ] || continue
  exe=$(readlink "/proc/$pid/exe" 2>/dev/null)
  case "$exe" in
    */sshd) echo "PID=$pid exe=$exe cmdline=$(cat /proc/$pid/cmdline 2>/dev/null | tr '\0' ' ')" ;;
  esac
done
```

**告警**：非标准端口上出现 sshd 二进制链接 = 高度疑似 SSH 后门。

---

## 模块 10：Webshell 扫描

扫描 `/var/www` 和 `/www/wwwroot`（或用户指定路径）下的 `.php` / `.jsp` 文件：

```bash
# PHP webshell 特征
WEBSHELL_PHP='array_map\(|pcntl_exec\(|proc_open\(|popen\(|shell_exec\(|passthru\(|base64_decode\s?\(|gzinflate|\(\$\$\w+|eval?\(|assert\('
find /var/www /www/wwwroot -type f -name "*.php" -exec grep -Pl "$WEBSHELL_PHP" {} + 2>/dev/null

# JSP webshell 特征
WEBSHELL_JSP='Runtime.getRuntime\(\).exec\(request'
find /var/www /www/wwwroot -type f -name "*.jsp" -exec grep -Pl "$WEBSHELL_JSP" {} + 2>/dev/null
```

**输出要点**：命中文件路径 + 匹配行内容。

---

## 模块 11：已知漏洞检查

逐项检查当前系统组件版本是否受已知 CVE 影响：

| 检查项 | 命令 | 受影响版本 |
|---|---|---|
| Redis 未授权 | `find / -name redis.conf -exec grep "# requirepass" {} +` | 注释掉 requirepass |
| Redis 弱口令 | `grep "^requirepass" <redis.conf>` | admin123/123456/password 等 |
| CVE-2018-15473 (OpenSSH 用户名枚举) | `sshd -V 2>&1` | ≤ 7.7 |
| CVE-2024-6387 (OpenSSH RCE) | 同上 | 8.5–9.7 |
| CVE-2021-3156 (Sudo 提权) | `sudo -V` | 1.8.2–1.8.31, 1.9.0–1.9.5 |
| CVE-2023-22809 (Sudo 提权) | 同上 | ≤ 1.9.12 |
| CVE-2024-3094 (XZ 投毒) | `xz --version` | 5.6.0, 5.6.1 |
| CVE-2016-5195 (Dirty COW) | `uname -r` | 内核 < 4.8.3 |
| CVE-2022-0847 (Dirty Pipe) | `uname -r` | 内核 5.8–5.16 |

**输出要点**：表格呈现——组件、当前版本、CVE 编号、风险状态（安全/受影响）。

---

## 模块 12：安全基线评估

按等保要求逐项检查：

```bash
# 1. 身份鉴别：空密码检查
awk -F: '($2=="" || $2=="!") {print $1}' /etc/shadow 2>/dev/null

# 2. 密码策略
grep '^PASS' /etc/login.defs

# 3. 登录失败锁定
grep '^auth' /etc/pam.d/system-auth /etc/pam.d/common-auth 2>/dev/null

# 4. 远程传输：SSH vs Telnet
systemctl is-active sshd 2>/dev/null; systemctl is-active telnet 2>/dev/null

# 5. 文件权限
ls -l /etc/passwd /etc/shadow

# 6. 审计服务
systemctl is-active auditd 2>/dev/null

# 7. 超时锁定
grep TMOUT /etc/profile 2>/dev/null

# 8. 访问控制
cat /etc/hosts.allow /etc/hosts.deny 2>/dev/null

# 9. 最小安装：多余端口/服务
ss -tulpn | awk '/LISTEN/'
```

**输出要点**：每项标注 ✅ 合规 / ❌ 不合规 / ⚠️ 需人工确认，附整改建议。

---

## 模块 13：Rootkit 查杀（可选）

若系统已安装 `chkrootkit` 或 `rkhunter`：

```bash
chkrootkit 2>/dev/null | grep -E 'INFECTED|Vulnerable|Warning'
rkhunter --check --sk 2>/dev/null | grep -E 'Warning|Rootkit|suspicious'
```

若未安装，提示用户安装后重新执行。

---

## 报告输出

将所有模块结果汇总为一份 Markdown 报告，使用 `sys_write` 写入 `output/` 目录：

```
文件名: output/security-audit-<YYYY-MM-DD-HHmm>.md
```

报告结构：
1. **摘要**：扫描时间、主机信息、风险等级总评（高/中/低）
2. **各模块详情**：按上述 13 个模块依次呈现
3. **风险汇总**：所有告警项汇总为表格（模块、风险描述、严重级别、建议措施）
4. **整改建议**：按优先级排列

使用 `task_plan` 规划执行步骤（≥3 步），每完成一个模块用 `task_update` 更新进度。
