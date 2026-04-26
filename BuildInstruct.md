# BuildInstruct — 체인지로그/패치노트 운영 가이드

이 문서는 **다음에 이 리포를 건드리는 에이전트와 사람**을 위한 운영
지침이다. 새 빌드가 나올 때마다 이 순서만 따라 하면 된다.

## TL;DR (AI 에이전트용)

- **소스 경로**: `/root/personaOS/` ← 수정은 여기서만.
- **라이브 사이트**: `https://docs.persona.lxy.rest/`
  (서빙 디렉터리 `/var/www/personaos/`, 건드리지 말 것).
- **배포 방식**: systemd watcher + 1분 타이머가 자동 rsync. 수동 배포 불필요.
- **즉시 공개**할 때: `changelog/NNNN-<codename>.md`에 `publishAt` 없이 쓰면
  다음 sync(≤60초)에 공개. 그 시점에 워킹트리 전체가 자동 커밋+푸시되는 건
  **아니다** — draft 승격 이벤트가 있을 때만 자동 git이 돌아간다.
- **예약 공개**할 때: `changelog/_drafts/NNNN-<codename>.md`에 저장하고
  `publishAt: 2026-05-15T21:00:00+09:00` 같은 시각을 적어두면 도달 시 자동
  승격 + 전체 워킹트리 커밋/푸시까지 진행된다 (§2.5 참고).
- **스포일러**: 본문 중 `||revealAt=2026-05-15T21:00:00+09:00|| 비밀 텍스트 ||`
  로 감싸면 그 시각 전에는 **서버가 해당 텍스트를 아예 클라이언트에 내려보내지
  않는다**. DevTools로도 볼 수 없음.
- **하지 말 것**: `/var/www/personaos/` 직접 편집, 수동 rsync, nginx/certbot
  재시작, `manifest.json` 손 편집, 예약 draft를 `_drafts/` 밖으로 꺼내 미리
  커밋하는 일.
- **빌드에 포함시킬 것 = OS 자체의 변경만**: 체인지로그 항목은 **OS
  (kernel / bootloader / libs/shared / toolchain / boot protocol)** 의
  수정사항만 기록한다. 문서 뷰어(`docs/patch-notes/`), 배포 스크립트,
  웹 UI 튜닝, README, 이 `BuildInstruct.md` 자체 등 **인프라·문서 변경**
  은 **빌드 번호를 올리지 않고 `Cargo.toml` 버전도 건드리지 않는다**.
  그런 변경은 그냥 커밋만 하면 끝이다. (§2.4 참고.)

---

## 1. 리포 구조 (관련 부분만)

```
changelog/
├── NNNN-<codename>.md          빌드 하나 = 파일 하나 (공개됨)
├── _drafts/                    예약 공개 대기 (gitignore됨 — git에서 안 보임)
│   └── NNNN-<codename>.md      publishAt 도달 시 상위로 자동 승격
└── manifest.json               웹 뷰어가 읽는 인덱스 (생성물)

docs/patch-notes/               정적 웹 뷰어 — 건드릴 필요 없음
├── index.html
├── style.css
├── md.js                       YAML frontmatter + Markdown 파서
└── app.js                      카운트다운/스포일러 렌더 포함

tools/
└── gen-changelog-manifest.py   changelog/ → manifest.json + 공개 게이팅
                                (에이전트가 직접 실행할 필요 없음)
```

## 1a. 배포 인프라 (이 서버에 설치되어 있음, 읽기 전용 참고)

```
/root/personaOS/                  소스. 에이전트가 편집하는 곳.
         │   (파일 저장 시 inotify 트리거)
         ▼
/usr/local/bin/personaos-sync     rsync + 자동 git 스크립트
         │   systemd 실행
         ▼
/var/www/personaos/               nginx가 서빙하는 디렉터리
         │
         ▼
nginx (TLS via Let's Encrypt)  →  https://docs.persona.lxy.rest/
```

systemd 유닛들:
- `/etc/systemd/system/personaos-sync.service` — 실제 sync 작업
- `/etc/systemd/system/personaos-sync.path` — 파일 변경 감시 (inotify)
- `/etc/systemd/system/personaos-sync.timer` — **1분** fallback 타이머
  (예약 공개 시각 도달 판정을 주도)
- `certbot.timer` — TLS 인증서 자동 갱신 (OS 기본)

추가 상태 파일:
- `/var/lib/personaos-sync/promoted.txt` — 이번 sync에서 승격된 draft 목록.
  비어있지 않을 때만 자동 git commit/push가 실행된다.

이 인프라는 이미 **설치·활성화**되어 있다. 에이전트는 다시 설치하지
않는다.

---

## 2. 패치노트(체인지로그) 작성 규칙

### 2.1 파일명

`changelog/NNNN-<codename>.md` (즉시 공개) 또는
`changelog/_drafts/NNNN-<codename>.md` (예약 공개).

- `NNNN` — 빌드 번호, **4자리 zero-pad**, 단조 증가. 예: `0001`, `0002`, `0013`.
- `<codename>` — 소문자, 공백 대신 `-`. 예: `ripple`, `north-star`.
- 한 빌드 = 한 파일. 기존 파일은 **수정하지 않고**, 새 파일을 만든다.
  (오타 수정 같은 소급 변경은 예외.)

### 2.2 파일 포맷

**반드시** 아래의 YAML frontmatter로 시작한다. 뷰어와 manifest 스크립트가
이 필드들을 읽는다.

```markdown
---
build: 2
codename: North Star
version: 0.0.3
date: 2026-05-10
publishAt: 2026-05-10T21:00:00+09:00   # 선택 — 예약 공개용
summary: 한 줄 요약. 리스트 카드에 그대로 보여짐.
tags: [kernel, tooling]
---

## Highlights

- 짧은 불릿 3~6개. "무엇이 달라졌나" 한눈 요약.

## Changes

### <컴포넌트>

- **<제목>** — 무엇을, 왜. 구현 상세는 커밋에 맡기고 의도를 적는다.

### <다른 컴포넌트>

- ...

## Notes

마이그레이션·알려진 이슈·다음 빌드 예고 등 자유 서술. 생략 가능.
```

#### frontmatter 필드

| 필드 | 타입 | 필수 | 설명 |
|------|------|------|------|
| `build` | 정수 | ✅ | 파일명의 `NNNN`과 동일한 숫자. 정렬 키. |
| `codename` | 문자열 | ✅ | 릴리스 코드네임. 대소문자/공백 OK. |
| `version` | 문자열 | ✅ | SemVer. `Cargo.toml`의 `workspace.version`과 맞춘다. |
| `date` | `YYYY-MM-DD` | ✅ | 릴리스 날짜. |
| `summary` | 문자열 | ✅ | 한 줄 요약(120자 이내 권장). |
| `tags` | 문자열 배열 | 선택 | `[bootloader, kernel, ...]` — 상세 뷰 상단에 태그로 표시. |
| `publishAt` | ISO 8601 | 선택 | **예약 공개 시각.** 있으면 draft. 없으면 즉시 공개. |

> **주의**: frontmatter 값에 콜론(`:`)이 들어가야 할 때는 큰따옴표로 감싼다.
> 예: `summary: "fix: blah"`.
> `publishAt`은 타임존 포함 ISO 8601(`+09:00`, `Z` 등) 필수. 타임존
> 생략 시 UTC로 해석된다.

#### 본문 규칙

- 섹션은 `## Highlights` / `## Changes` / `## Notes` 세 개를 기본으로
  쓴다. 내용이 없으면 해당 섹션은 생략.
- `### <컴포넌트>` 수준으로 영역을 나눈다. 예: `bootloader`, `kernel`,
  `libs/shared`, `tooling`, `docs`.
- 불릿은 `**제목** — 본문` 형식을 권장한다. 제목만 굵게.
- 긴 문장은 다음 줄로 들여써서 이어 써도 된다 (파서가 list
  continuation을 접어준다).
- 이미지/GIF·외부 링크·표 같은 CommonMark 기능은 `md.js`가 지원하는
  범위 안에서만 쓴다(지원: ATX 헤딩, 불릿 리스트, `**bold**`,
  `*italic*`, `` `code` ``, 링크, fenced code block, hr, blockquote).

### 2.3 버전 정책

- **패치** (버그 고침만) → `0.0.X` 증가.
- **마이너** (기능 추가, 후방 호환) → `0.X.0` 증가.
- **메이저 / 부트프로토콜 브레이크** → `X.0.0` 증가 **그리고**
  `BOOT_INFO_VERSION`도 올린다 (`libs/shared/src/boot_info.rs`).

### 2.4 빌드에 포함시킬 범위 — OS 수정사항만

체인지로그/패치노트는 **personaOS 자체의 변경**만 추적한다. 인프라나
문서가 바뀌었다고 빌드 번호를 올리지 않는다.

#### 빌드 엔트리를 만들 변경 (= OS 수정사항)

- `kernel/` — 커널 코드, 드라이버, 스케줄러, syscall, mm 등
- `bootloader/` — personaboot 코드
- `libs/shared/` — `BootInfo`·boot protocol 등 OS ABI
- `Cargo.toml` / `rust-toolchain.toml` — 버전/툴체인이 OS 빌드에
  영향 주는 경우
- `kernel/x86_64-personaos-kernel.json` 등 타깃 스펙
- 커널/부트로더가 쓰는 링커 스크립트, build.rs
- OS 이미지 아티팩트 생성에 영향 주는 Makefile 타깃 (실제 산출물이
  달라질 때)

#### 빌드 엔트리를 만들지 **않는** 변경 (커밋만 하면 끝)

- `docs/patch-notes/` — 웹 뷰어의 HTML/CSS/JS, 레이아웃 튜닝,
  스크롤·폰트·색상 조정
- `changelog/` 그 자체 (오타 수정 등)
- `tools/gen-changelog-manifest.*`, `personaos-sync` 등 배포 스크립트
- `tools/mkdisk.sh`/`run-qemu.sh` 중 **OS 산출물을 바꾸지 않는**
  실행 편의 플래그 조정
- `README.md`, `BuildInstruct.md` 등 문서
- `.gitignore`, CI 설정
- systemd 유닛, nginx 설정, 인프라성 변경

이런 항목은 **`Cargo.toml`의 `workspace.package.version`도 건드리지
않는다.** `workspace.version`은 "OS 자체의 버전"이고, 문서/인프라가
달라졌다고 같이 올려선 안 된다.

판단 기준: **부팅된 personaOS 바이너리의 동작·크기·ABI가 달라지는가?**
- Yes → 빌드 엔트리 작성, `version` bump, §3 절차.
- No → 평범한 git 커밋만 하고 끝. 체인지로그 생성 금지.

애매하면 엔트리를 **만들지 않는** 쪽을 택한다. 사람이 나중에 필요하면
만들 수 있지만, 잘못 만들어진 엔트리는 리비전 히스토리를 오염시킨다.

### 2.5 예약 공개 + 스포일러

예약 공개와 인라인 스포일러는 **서버 사이드 게이트**로 동작한다.
클라이언트(브라우저)가 받는 HTML/JSON에는 **공개 시각 이후의 텍스트만**
포함된다. F12로 DOM을 조작해도 볼 수 없다.

#### 예약 공개 (draft)

1. 파일을 `changelog/_drafts/NNNN-<codename>.md`에 저장한다.
   `_drafts/`는 `.gitignore`에 등록되어 있어 git에도 안 보인다.
2. frontmatter에 `publishAt: 2026-05-15T21:00:00+09:00`처럼 시각을 적는다.
3. 저장하는 순간 웹사이트에는 **"Scheduled release · build #NNN · 카운트다운"**
   카드가 나타난다 (codename/summary/본문은 노출되지 않음 — 서버에 파일이
   /var/www 에 올라가지 않으므로).
4. `publishAt` 도달 시 (최대 60초 지연):
   - generator가 파일을 `_drafts/`에서 `changelog/`로 이동시킨다.
   - 본문이 `/var/www/personaos/changelog/`에 등장하고 카드가 일반 릴리스로
     바뀐다.
   - **자동으로 `git add -A && git commit && git push` 가 실행**된다.
     커밋 메시지는 `changelog: auto-publish build N — Codename`.
     이때 워킹트리에 남아있던 **다른 변경(`kernel/`, `Cargo.toml` 등)도
     같이 커밋된다.** 예약 시점에 함께 푸시되길 바라지 않는 코드 변경은
     그 전에 개발자가 **직접 커밋해서 빼두는 것이 안전하다.**

#### 인라인 스포일러

본문 안에 다음과 같이 쓰면 된다:

```markdown
이번 분기 목표는 ||revealAt=2026-05-15T21:00:00+09:00|| Secret Feature X || 입니다.
```

- `revealAt` 이전: 서버가 이 블록 전체를 `<span class="spoiler-locked" …></span>`
  로 치환한 뒤 내려보낸다. **원문 텍스트는 서버 외부로 나가지 않는다.**
- `revealAt` 이후: 마커만 떨어져 나가고 인라인 텍스트가 그대로 공개된다.
- 웹에선 대각선 스트라이프 박스가 보이고, 클릭이나 DevTools로 열어봐도
  평문이 없다.
- `revealAt` 형식은 `publishAt`과 동일한 ISO 8601. 타임존 포함 권장.
- 한 문서에 여러 개 써도 된다. 같은 `revealAt` 를 공유할 수도 있다.

#### 자동 커밋/푸시의 동작 원칙

- **트리거**: draft 승격이 발생한 sync 실행. 평시 sync(매 1분)는 커밋을
  만들지 않는다.
- **범위**: 워킹트리 전체 (`git add -A`). `.gitignore`로 제외된 것(`target/`,
  `build/`, `_drafts/` 등)은 제외.
- **저자**: `personaOS autopublisher <autopublish@persona.lxy.rest>`
  로 고정. 사람 커밋과 구별된다.
- **푸시 실패**: 푸시가 실패해도 커밋은 로컬에 남는다 (`git push failed`
  로그만 남고 스크립트는 0 exit). 다음 성공 sync에서는 예약 이벤트가
  없으면 자동 푸시가 돌지 않으므로, 실패 후 복구는 사람이 `git push`로
  처리한다.
- **draft 단계에서 git 히스토리에 흔적**: `_drafts/` 전체가 ignore이므로
  남지 않는다. 승격된 뒤에야 git이 인지한다.

---

## 3. 패치노트를 사이트에 올리는 절차

> **중요 — AI 에이전트가 반드시 알아야 할 배포 모델**
>
> 이 리포의 **소스는 `/root/personaOS`이고, 라이브 사이트는
> `/var/www/personaos`**다. 둘은 **systemd가 자동으로 동기화**한다.
> 에이전트가 할 일은 **`/root/personaOS` 안의 파일을 수정하는 것뿐**.
> rsync·복사·재시작을 수동으로 돌리지 **말 것**. 자동화는 아래와
> 같이 돌아간다:
>
> - `personaos-sync.path` — `/root/personaOS/changelog/`와
>   `/root/personaOS/docs/patch-notes/`를 inotify로 감시. 파일이
>   바뀌면 즉시 `personaos-sync.service` 실행.
> - `personaos-sync.timer` — **1분**마다 실행. 예약 공개 시각 판정의
>   기준.
> - `personaos-sync.service` → `/usr/local/bin/personaos-sync` —
>   generator 실행 → changelog 트리를 `/var/www/personaos/changelog`에
>   직접 쓰기 → 나머지 리포 rsync → draft 승격 있었으면 자동 git 커밋+푸시.
>
> **결과**: 파일을 저장하는 순간 몇 초 안에
> `https://docs.persona.lxy.rest/`에 반영된다. 수동 개입 없음.

**체크리스트 — 에이전트는 이 순서를 그대로 따른다.**

1. **체인지로그 파일 작성**
   - 즉시 공개: `/root/personaOS/changelog/NNNN-<codename>.md` 생성.
   - 예약 공개: `/root/personaOS/changelog/_drafts/NNNN-<codename>.md`
     생성 + frontmatter에 `publishAt` 필드. §2 규칙 준수.
2. **코드상의 버전 업데이트**
   - `/root/personaOS/Cargo.toml`의 `workspace.package.version`을
     frontmatter `version`과 동일하게 바꾼다.
3. **끝.** — 저장하는 순간 watcher가 동기화한다. 다음 단계는 모두
   **자동**이다 (참고용으로만 나열):
   - manifest 재생성 — `personaos-sync`가 generator를 자동 호출.
   - 라이브 디렉터리로 복사 — rsync가 자동 실행.
   - 웹 반영 — 뷰어가 새 manifest를 fetch해서 새 카드를 렌더.
   - (예약 공개의 경우) `publishAt` 시각에 draft 승격 + 자동 git 커밋/푸시.

### 3.1 반영 확인

```
curl -s https://docs.persona.lxy.rest/changelog/manifest.json | head
```
방금 쓴 빌드의 엔트리가 최상단에 있으면 완료.
예약 공개된 빌드는 `"locked": true` 로 나타나며 `file: null`이다.

### 3.2 Git 커밋

- **즉시 공개**: 자동 푸시가 돌지 않는다. 평소대로 사람이 커밋/푸시한다.
  ```
  git add changelog/NNNN-<codename>.md changelog/manifest.json Cargo.toml
  git commit -m "changelog: build NNNN — <codename>"
  git push
  ```
- **예약 공개**: draft는 `.gitignore`로 숨겨지므로 예약 단계에서는 아무
  커밋도 만들지 않는다. `publishAt` 도달 시 `personaos-sync`가 알아서
  전체 워킹트리를 커밋/푸시한다. 그 전에 섞여서 나가면 곤란한 코드
  변경은 **개발자가 미리 따로 커밋**해두는 것이 안전하다.

### 3.3 동기화가 의심될 때 (트러블슈팅)

```
systemctl status personaos-sync.path personaos-sync.timer
journalctl -u personaos-sync.service -n 30 --no-pager
systemctl start personaos-sync.service     # 즉시 강제 실행
cat /var/lib/personaos-sync/promoted.txt   # 마지막 승격 이벤트 (비어있음 = 정상)
```

`/var/www/personaos/changelog/`가 `/root/personaOS/changelog/`와
일치하지 않으면 watcher가 멈춘 것. `systemctl restart
personaos-sync.path`.

자동 푸시가 안 됐을 때:
- generator가 draft를 승격했는지: `journalctl -u personaos-sync.service`
  에서 `promoted draft →` 줄을 찾는다.
- 커밋이 생겼는지: `git log --author=autopublisher -1`.
- 푸시가 실패했는지: 위 로그에서 `git push failed`를 본다. 실패 시 리모트
  인증/충돌 문제이므로 사람이 `cd /root/personaOS && git push`.

---

## 4. manifest.json 구조 (참고)

스크립트가 생성하므로 **손으로 편집하지 않는다.** 구조는 이렇다:

```json
[
  {
    "file": null,
    "build": 3,
    "locked": true,
    "publishAt": "2026-05-15T12:00:00Z"
  },
  {
    "file": "0002-north-star.md",
    "build": 2,
    "codename": "North Star",
    "version": "0.0.3",
    "date": "2026-05-10",
    "summary": "..."
  },
  { "file": "0001-ripple.md", "build": 1, ... }
]
```

- 공개된 빌드: `file`, `codename`, `version`, `date`, `summary` 채워짐.
- 예약 중 빌드: `locked: true`, `file: null`, `publishAt`만 존재. 다른
  필드는 전송되지 않는다.
- `build` 내림차순 정렬이 보장된다.

---

## 5. 자주 하는 실수

- **`/var/www/personaos/`를 직접 수정.** → 다음 sync에서 덮어써진다.
  반드시 `/root/personaOS/`에서만 수정.
- **예약 draft를 `changelog/` 아래에 바로 둠.** → 본문이 `/var/www`로
  즉시 복사되어 공개 예정이라는 게 무의미해진다. 반드시 `_drafts/`에 둔다.
- **`publishAt`에 타임존 생략.** → UTC로 해석된다. KST/JST라면 `+09:00`
  을 꼭 붙인다.
- **`0001-ripple.md`를 직접 고치고 끝.** → 옛 빌드는 불변.
  새 파일을 만들어야 한다.
- **`build` 번호와 파일명 `NNNN`이 불일치.** → 정렬이 꼬임.
- **`manifest.json`을 손으로 편집.** → 다음 sync에서 덮어써짐.
- **frontmatter 바깥 위쪽에 텍스트가 있음.** → 파서가 frontmatter를
  인식 못 함. 파일 **첫 줄**이 `---`이어야 한다.
- **CommonMark 기능 중 표/이미지 사용.** → `md.js`가 지원 안 함.
- **`Cargo.toml` 버전 업데이트 누락.** → frontmatter `version`과
  실제 빌드 버전이 어긋남.
- **OS가 아닌 변경으로 빌드 엔트리를 만듦.** → 예: 웹 뷰어 CSS 튜닝,
  README 수정, 배포 스크립트 조정으로 `changelog/NNNN-*.md`를 생성하고
  `Cargo.toml`의 `workspace.version`까지 올리는 것. OS 바이너리가
  전혀 안 바뀌었는데 릴리스 히스토리에 가짜 빌드가 남는다. §2.4의
  범위 규칙을 따른다. 인프라/문서 변경은 **평범한 git 커밋만** 하면 된다.
- **예약 공개 직전에 미완성 코드 변경을 워킹트리에 쌓아둠.** → draft
  승격 시 자동 커밋이 그 변경까지 끌고 간다. 섞이면 안 되는 건 미리
  따로 커밋해둔다.
- **배포 안 됐다고 nginx 재시작.** → 필요 없다. sync 로그부터 확인
  (`journalctl -u personaos-sync.service -n 30`).

---

## 6. 새 템플릿 (복사용)

### 즉시 공개 — `changelog/NNNN-<codename>.md`

```markdown
---
build: 
codename: 
version: 
date: 
summary: 
tags: []
---

## Highlights

- 

## Changes

### <컴포넌트>

- **<제목>** — 

## Notes

```

### 예약 공개 — `changelog/_drafts/NNNN-<codename>.md`

```markdown
---
build: 
codename: 
version: 
date: 
publishAt: 2026-05-15T21:00:00+09:00
summary: 
tags: []
---

## Highlights

- 

## Changes

### <컴포넌트>

- **<제목>** — 

예고편 샘플: ||revealAt=2026-05-15T21:00:00+09:00|| 여기 적힌 건 revealAt 전엔 절대 클라이언트에 안 간다 ||

## Notes

```
