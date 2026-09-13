# eml-linux — «всё есть сообщение»

Linux-стек, где атомарный объект — не файл, а **RFC 822 / MIME документ (`.eml`)**.
Один формат закрывает то, что в Linux размазано по десятку: сервисы, логи, пакеты,
IPC и конфигурацию. Надстройка — нечёткие клеточные автоматы и теория игр как
губернатор ресурсов.

> «Everything is a file» → **«everything is a message»**.

## Состав (всё уже работает и протестировано)

| Крейт | Роль | Что делает |
|---|---|---|
| **emlcore** | субстрат | RFC822-запись, SHA-256 hash-chain, append-only spool, парсер юнитов |
| **emlinit** | init / супервизор | юниты = `.eml`; рестарт-политики, зависимости, graceful shutdown, управление через `.eml`-шину |
| **emllog** | замена journald | `tail` / `verify` / `grep` / `stats` по хеш-цепочке событий |
| **emlpkg** | пакетный менеджер | пакет = один `.eml`; транзакционная установка + откат через дельты |
| **emlfs** | FUSE-ФС | монтирует каталог `.eml` как read-only ФС (`raw`/`headers`/`body`/`payload.json`) |
| **emlca** | губернатор ресурсов | нечёткий КА (диффузия нагрузки) + VCG/Shapley/Nash; решения — `.eml` |
| **emlbus** | pub/sub шина | сообщения = `.eml`; доставка по `To:` (entity/topic/`*`), consumed -> `.done` |
| **emlnet** | CRDT-меш | multi-writer LWW-репликация по TCP; op-id дедуп; сходимость за один обмен |
| **emlsign** | крипто-провенанс | ed25519-подпись `.eml` (headers+body), verify, пиннинг ключа |
| **emlcron** | планировщик | интервальные `.eml`-задачи; каждый запуск — событие в цепочке |
| **emlwatch** | inotify-мост | события ФС → поток `.eml` (CREATE/MODIFY/DELETE/...) |

## Формат

Каждое событие/юнит — RFC822-документ. Совместим с EML-IPC (`mime-os/docs/FORMAT.md`):
`From` / `To` / `X-EMLBox-Msg` / `Content-Type`, имя файла `<seq>.<id>.msg.eml`.
`emlbox ipc list <spool>` и `emlbox tagdb` читают наши записи нативно.

Каждое событие несёт `X-Emllog-Prev-Hash` / `X-Emllog-Hash` — цепочку нельзя
подделать незаметно (`emllog verify`).

## Быстрый старт

```sh
cargo build --release
BIN=target/release

# 1. init: запустить сервисы из .eml-юнитов
$BIN/emlinit list  --units examples/units
$BIN/emlinit run   --units examples/units --spool run/emlinit

# 2. управление через .eml-шину (из другого терминала)
$BIN/emlinit send  --bus run/emlinit/control --event STOP

# 3. лог как проверяемая цепочка
$BIN/emllog --spool run/emlinit tail
$BIN/emllog --spool run/emlinit verify

# 4. пакет = .eml
$BIN/emlpkg build examples/units /tmp/p.eml --id demo --version 1.0.0
$BIN/emlpkg install /tmp/p.eml --root /tmp/root --log /tmp/pklog
$BIN/emlpkg rollback --root /tmp/root --log /tmp/pklog

# 5. FUSE
mkdir -p /tmp/mnt && $BIN/emlfs run/emlinit /tmp/mnt &
ls /tmp/mnt; cat /tmp/mnt/*/headers; fusermount3 -u /tmp/mnt

# 6. КА + теория игр
$BIN/emlca diffuse --cells "255,0,0,0,0,0,0,0" --decay 1 --steps 5
$BIN/emlca vcg     --bids "10,7,3,2" --slots 2
$BIN/emlca shapley --players "A,B,C" --v "A=1,B=2,C=0,AB=5,AC=4,BC=3,ABC=9"
$BIN/emlca nash    --candidates "10,1;6,6;1,10" --d "0,0"
```

## Юнит сервиса

```text
From: <web@eml.local>
Subject: web server
X-Entity-ID: web
Content-Type: application/json

{"exec":["/usr/bin/webserver","-p","8080"],
 "restart":"on-failure","restart_sec":1,"after":["network"]}
```

## Интеграция с текущим init (dinit)

Сейчас на машине PID 1 — **dinit**. Готовый путь:

```sh
# запустить emlinit как супервизор под dinit (user-инстанс):
sh deploy/install-user-service.sh
dinitctl start emlinit
```

Полная замена PID 1 (как отдельный init на базе `emlcore`+`emlinit`) требует
правки boot-конфигурации под root — см. `deploy/`.

## Тесты

```
cargo test     # 30 tests: emlcore 10, emlca 5, emlsign 4, emlnet 3, emlwatch 3, emlbus 2, emlcron 2, emlfs 1
```
Плюс e2e-сценарии: старт/exit-коды/рестарт/graceful shutdown, детект подмены
тела события (`BROKEN at seq N`), установка/откат пакета, живое FUSE-монтирование.

## Дальше

- **eml-bus** как общесистемная шина (замена dbus-части): подписка по `To:`.
- **eml-net**: multi-writer CRDT-синк (UUCP 2.0) — формат уже тот же.
- **eml-udev**: устройства как tagdb-сущности.
- **CA-губернатор в планировщике**: диффузия нагрузки по NUMA-решётке.
- Полная замена dinit/systemd: init = реплей загрузочной `.eml`-цепочки.

## Шина и сеть

```sh
# pub/sub: сообщения — .eml, доставка по To:
$BIN/emlbus pub  --bus /tmp/bus --from a --to worker --event JOB --body '{"n":1}'
$BIN/emlbus list --bus /tmp/bus --to worker
$BIN/emlbus sub  --bus /tmp/bus --to worker --once

# CRDT-меш (UUCP 2.0): два узла сходятся за один обмен
$BIN/emlnet put   --store /tmp/a --writer A --key x --value 1
$BIN/emlnet serve --store /tmp/a --addr 127.0.0.1:9099 &
$BIN/emlnet sync  --store /tmp/b --peer 127.0.0.1:9099
$BIN/emlnet get   --store /tmp/b
```

## Архитектура

```
L5  game theory   — VCG / Shapley / Nash          (emlca)
L4  fuzzy CA      — диффузия нагрузки, async tick  (emlca)
L3  .eml сеть     — CRDT multi-writer sync / TCP   (emlnet)
L2  .eml шина     — pub/sub, hash-chain лог        (emlbus, emlinit, emllog)
L1  Linux         — init / пакеты / FUSE-ФС        (emlinit, emlpkg, emlfs)
L0  формат        — RFC822 + SHA-256 цепочка       (emlcore)
```

## Провенанс, расписание, наблюдение

```sh
# ed25519: подписать и проверить любой .eml
$BIN/emlsign keygen --key node.key
$BIN/emlsign sign   --key node.key --in msg.eml --out msg.signed.eml
$BIN/emlsign verify --in msg.signed.eml            # OK
$BIN/emlsign verify --in msg.signed.eml --pub <HEX> # пиннинг подписанта

# расписание: задачи как .eml, каждый запуск — событие в цепочке
$BIN/emlcron list --jobs jobs/
$BIN/emlcron run  --jobs jobs/ --spool run/cron

# наблюдение: inotify -> .eml
$BIN/emlwatch --dir ./watched --spool run/watch --mask create,modify,delete
```
