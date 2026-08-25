# Browser Use 鍐呭瓨璇垽淇涓庡彲瑙佹€х瓥鐣ヤ笁鎬佸寲锛?026-08-19锛?
## 1. 鑳屾櫙涓庣敤鎴锋姤鍛?
鐢ㄦ埛鎶ュ憡锛氥€宎gent 瑙﹀彂 browser use 浣跨敤娴忚鍣ㄧ殑鏃跺€欏緢瀹规槗瑙﹀彂鍐呭瓨闂鑰屽叧闂紝
浣跨敤浣撻獙闈炲父宸€嶃€?
鎺掓煡缁撹锛?*涓嶆槸鍐呭瓨娉勬紡锛屾槸璇垽 + 绉掓潃**銆備竴娆℃櫘閫氱殑鍗?Agent 娴忚浼氳瘽锛屽湪椤甸潰
鍔犺浇瀹屾垚鍚庣害涓€涓噰鏍峰懆鏈燂紙5 绉掞級鍐呭氨浼氳寮哄埗鍏抽棴锛屽苟涓旇褰撲綔銆岀敤鎴峰叧闂簡娴忚鍣ㄣ€?涓婃姤缁?Agent銆?
鍚屾椂纭畾浜嗙浜岄」闇€姹傦細闈欓粯/鍓嶅彴鐨勯€夋嫨涓嶅簲璇ョ敱鐢ㄦ埛涓烘墍鏈夊満鏅竴娆℃€ф寚瀹氾紝鑰屽簲鐢?娴忚鍣ㄦ寜鍦烘櫙鍒ゆ柇锛岀敤鎴峰彧鎻愪緵涓€涓厹搴曞€惧悜銆?
## 2. 鏍瑰洜锛堜笁椤圭己闄峰彔鍔狅級

### 2.1 搴﹂噺鏈韩铏氶珮绾?1.7 鍊?
`sample_browser_resources` 鎶婃暣妫?Chromium 杩涚▼鏍戠殑 `sysinfo::Process::memory()`
绱姞銆傚湪 Windows 涓婅鍊兼槸 `WorkingSetSize`锛堝伐浣滈泦锛夛紝**鍖呭惈涓庡厔寮熻繘绋嬪叡浜殑椤?*锛?`chrome.dll` 绛夊叡浜暅鍍忔槧灏勮繘姣忎釜瀛愯繘绋嬶紝绱姞绛変簬鎶婅繖浜涢〉鎸夎繘绋嬫暟閲嶅璁＄畻銆?
鏈満涔濊繘绋?Chrome 瀹炴祴锛?
| 鎸囨爣 | 鍚堣 |
| --- | ---: |
| `WorkingSet64`锛堝師瀹炵幇鎵€鐢級 | 696.3 MiB |
| `PrivateMemorySize64`锛堢鏈夋彁浜ら噺锛?| 413.1 MiB |
| 琚噸澶嶈绠楃殑鍏变韩椤?| 283 MiB锛?1%锛?|
| 铏氶珮鍊嶆暟 | 1.69脳 |

`sysinfo` 鍦?Windows 涓婂凡缁忔妸绉佹湁鎻愪氦閲忔毚闇蹭负 `Process::virtual_memory()`
锛堟槧灏勫埌 `PROCESS_MEMORY_COUNTERS_EX::PrivateUsage`锛夈€傜鏈夋彁浜ら噺鏄繘绋嬬嫭鍗犵殑锛?璺ㄨ繘绋嬬疮鍔犳墠鏄湁鏁堢殑銆?
**骞冲彴闄烽槺**锛氬湪 Linux/macOS 涓?`virtual_memory` 鏄櫄鎷熷湴鍧€绌洪棿澶у皬锛圴SZ锛夛紝
鐢ㄥ畠浼氭洿绯燂紱涓?`sysinfo` 涓嶆毚闇?PSS銆傚洜姝や慨姝ｅ繀椤?`#[cfg(windows)]` 闄愬畾锛?鍏朵粬骞冲彴淇濈暀 RSS锛屽叾鏍戞€婚噺浠嶆槸涓婄晫銆?
### 2.2 棰勭畻瑁呬笉涓嬩竴涓湡瀹炴祻瑙堝櫒

鍗曚换鍔＄嫭鍗犱竴涓?Host 鏃朵細琚綊鍥犳暣妫佃繘绋嬫爲锛屽叾涓寘鍚?Chromium 鐨勫浐瀹氬熀绾?锛坆rowser + GPU + network/storage + crashpad锛夛紝杩欓儴鍒嗗湪娓叉煋浠讳綍椤甸潰涔嬪墠灏卞瓨鍦ㄣ€?瀵圭収鍘熸潵鐨?1 GiB锛屼竴妫?*绌洪棽**鏍戝凡缁忛噺鍒扮害 700 MiB銆?
### 2.3 鍥炴敹鐬棿鍗囨。锛屼笖浼氭嬁璧颁换鍔″敮涓€鐨?lane

severity/confidence 鍔犻€熼」璁?streak=1 灏辫冻浠ヨ繘鍏ュ彲鍥炴敹妗ｏ紱鑰屼笌
`freeze_idle_lane_for_pressure` 涓嶅悓锛屽洖鏀惰矾寰?*娌℃湁**鏈€鍚庝竴鏉?lane 鐨勪繚鎶ゃ€?浜庢槸涓€涓湪涓ゆ宸ュ叿璋冪敤涔嬮棿绛夊緟妯″瀷鎬濊€冪殑 Agent锛堣繖鏄ぇ閮ㄥ垎鏃堕棿锛夛紝鍦ㄧ涓€涓?瓒呴绠楅噰鏍峰氨涓㈡帀浜嗘祻瑙堝櫒銆?
### 2.4 鎶ラ敊鐢╅攨缁欑敤鎴蜂笖绂佹閲嶈瘯

鍥炴敹鎶涘嚭 `LaneClosedByUser`锛?The browser lane was closed."锛宍retryable: false`锛夈€?
## 3. 宸插疄鏂界殑淇

| 鏀瑰姩 | 浣嶇疆 |
| --- | --- |
| Windows 褰掑洜鏀圭敤绉佹湁鎻愪氦閲?| `services.rs::process_tree_attributable_bytes` |
| `AUTOMATIC_TASK_MEMORY_BYTES` 1 GiB 鈫?2 GiB | `resource.rs` |
| `RESOURCE_SAVING_TASK_MEMORY_BYTES` 768 MiB 鈫?1.25 GiB | `resource.rs` |
| 鏂板 `TASK_RECLAIM_MIN_SUSTAINED_SAMPLES = 3` 婊炲洖涓嬮檺 | `hub.rs` |
| 浠诲姟鍞竴 lane 浠呮渶楂樻。鍙洖鏀?| `hub.rs::reclaim_over_budget_tasks` |
| 鏂板 `TaskMemoryReclaimed`锛坄retryable: true`锛屾槧灏?429锛?| `error.rs`銆乣browser_management.rs` |

### 3.1 涓ゆ潯涓嶅彲鏀惧鐨勪笉鍙橀噺

1. **婊炲洖涓嬮檺**锛氥€屾槀璐点€嶄笌銆屾硠婕忋€嶆槸涓や欢浜嬨€傛祻瑙堝櫒鍙互鍚堟硶鍦伴暱鏈熷仠鍦ㄩ珮浣?   锛堝嚑涓獟浣撳瘑闆嗘爣绛鹃〉锛夛紝鑰?streak 鍙湪鍥炶惤鍒伴绠椾互涓嬫墠娓呴浂鈥斺€旀病鏈夋粸鍥烇紝
   绋冲畾浼氳瘽浼氫笌澶辨帶浼氳瘽琚悓绛夋儵缃氥€?2. **鏈€鍚庝竴鏉?lane 淇濇姢**锛氬崟 lane 鏄?Agent 浠诲姟鏈€甯歌鐨勫舰鎬侊紝鍏虫帀瀹冪瓑浜庡叧鎺?   鐢ㄦ埛鐨勬暣涓祻瑙堝櫒銆?
**鐗瑰埆娉ㄦ剰**锛氭渶鍚庝竴鏉?lane 鐨勪繚鎶?*涓嶅緱**棰濆瑕佹眰 `severely_over`銆傞偅鏍蜂細璁?銆屽崟 lane + 鎸佺画涓害瓒呴檺銆嶆案涔呭厤鐤洖鏀讹紝鏄紡娲炶€岄潪淇濇姢銆傜埇鍒版渶楂樻。鏈韩灏辨槸淇濇姢銆?
## 4. 鍙鎬х瓥鐣ヤ笁鎬佸寲

### 4.1 涓€涓喅瀹氭€х害鏉?
鍙鎬у垏鎹?*涓嶆槸绐楀彛寮€鍏?*锛岃€屾槸鏇挎崲 Chromium Host 杩涚▼锛?`set_lane_visibility_for_user` 鈫?`set_lane_visibility_and_maybe_focus_once`
鈫?`transition_primary_visibility_locked` 鈫?Host 閲嶅惎銆?`HOST_RESTART_ATTEMPT_TIMEOUT` 涓?75 绉掞紝涓旂敱浜?Primary 鍚?lane 鍏变韩涓€涓鑼?Host锛屾浛鎹細鍦ㄦ柊 epoch 涓嬮噸缁?*鎵€鏈?*瀛樻椿鐨?Primary lane銆?
鍥犳锛?*寮€ lane 鏃跺喅瀹氭槸鍏嶈垂鐨勶紝杩愯涓啀鍐冲畾涓嶆槸銆?*

### 4.2 鍥涘眰璁捐

**绗?1 灞?路 鐢ㄦ埛鍋忓ソ锛堝厹搴曚笌纭竟鐣岋級**

| 鍙栧€?| 璇箟 |
| --- | --- |
| `headless` | 姘歌繙闈欓粯锛屽嵆浣块亣鍒伴渶瑕佺敤鎴蜂粙鍏ョ殑鏃跺埢 |
| `auto` | **鏂伴粯璁?*锛氬涓绘寜 lane 瑁佸喅 |
| `external` | 姘歌繙浠ョ湡瀹炵獥鍙ｅ惎鍔?Primary |

**绗?2 灞?路 妯″瀷鎰忓浘锛堝缓璁紝闈炴潈濞侊級**

宸ュ叿鎺ュ彈 `presentation`锛歚unattended`锛堥粯璁わ級/ `attended`銆傛ā鍨嬪彧琛ㄨ揪**鎰忓浘**锛?涓嶅緱鎸囧畾鏈哄埗锛涗紶 `headless`/`headful`/`external`/`visible` 绛夋満鍒惰瘝浼氳**鎷掔粷**
骞剁粰鍑烘敼鐢ㄦ剰鍥剧殑鎻愮ず锛岃€屼笉鏄潤榛橀檷绾т负渚嬭銆傝繖涓?`MODEL_IDENTITY_INPUT_FIELDS` 鎷掔粷妯″瀷鎸囧畾韬唤/妗ｆ鏄悓涓€鏉＄邯寰嬨€?
涓や釜涓婃姤鐐癸細寮€ lane 鏃讹紝浠ュ強鍦ㄨ繍琛屼腑 lane 涓婃淳鍙戞搷浣滃墠銆傚悗鑰呮墠鏄富鍦烘櫙鈥斺€旂湡瀹?娴佺▼鏄€屽鑸?鈫?鎾炰笂鐧诲綍澧?鈫?姝ゆ椂鎵嶉渶瑕佺敤鎴枫€嶃€?
**绗?3 灞?路 瀹夸富瑁佸喅锛堟潈濞侊級**

`resolve_lane_visibility(policy, intent, identity_mode)` 涓?`may_escalate_lane_to_headful(policy, intent, identity_mode, current, used)`
涓虹函鍑芥暟锛岀瓥鐣ヤ互鐪熷€艰〃褰㈠紡鍙鍙祴銆?
**绗?4 灞?路 鍗曞悜鍗囩骇**

- 浠?`auto` 浼氬崌绾э紱`headless` 宸插悜鐢ㄦ埛鎵胯涓嶅脊绐楋紝`external` 鏈氨鍙銆?- 浠?Primary锛氬彧鏈夊畠鎵胯浇鐢ㄦ埛鐪熷疄鐧诲綍鎬併€侫nonymous 鎾炵櫥褰曞搴旀姤
  `NeedsPrimaryIdentity`锛岃€屼笉鏄湪鏃犳硶鐧诲綍鐨勬。妗堜笂寮圭獥銆?- **鍙湞鍙鏂瑰悜**锛氳鐢ㄦ埛鑳界湅瑙佸苟鎺ユ墜鏄畨鍏ㄦ柟鍚戯紱鎶婄敤鎴锋鍦ㄧ洃鐫ｇ殑宸ヤ綔钘忚捣鏉?  鏄€忔槑鎬у€掗€€锛屽洜姝?*鏁呮剰涓嶆彁渚?*闄嶇骇璺緞銆?- 姣?lane 涓婇檺 `MAX_LANE_VISIBILITY_ESCALATIONS = 2`锛屽洜涓烘瘡娆￠兘鏄竴娆¤繘绋嬫浛鎹€?
### 4.3 瀹炴柦涓慨鎺夌殑涓や釜鐪熷疄缂洪櫡

1. **杩佺Щ浼氳绐楀彛澶嶆椿**銆倂2 鐨勬敞閲婂啓鏄庯細鏃犵増鏈爣璁扮殑 `external` 鍙兘鏄粠宸插簾寮冪殑
   `silent=false` **鎺ㄦ柇**鍑烘潵鐨勶紝v2 鐗规剰闃绘浜嗚繖绫荤姸鎬佸脊绐椼€傜涓€鐗堣縼绉绘棤鏉′欢淇濈暀
   `external`锛屼細璁╄繖浜涚敤鎴烽噸鏂拌寮圭獥銆傚凡鏀逛负鎸変笘浠ｅ尯鍒嗭細浠?**v2 鏍囪**璇佹槑鏄槑纭?   閫夋嫨鎵嶄繚鐣欙紱鏃犵増鏈竴寰嬭縼鍒?`auto`銆備粨搴撴棦鏈夋祴璇?   `..._unversioned_external_to_headless` 鎶撳埌浜嗚繖涓洖褰掋€?2. **鍗囩骇浼氬舰鎴愰噸鍚惊鐜?*銆傜涓€鐗堜粠 `config.headful` 璇诲彇褰撳墠鍙鎬э紝浣嗘寜 lane 鐨?   鍒囨崲**鏁呮剰涓嶆敼**瀹夎绾ч粯璁ゅ€硷紝浜庢槸宸插彲瑙佺殑 Host 琚垽涓洪潤榛橈紝姣忔涓婃姤閮藉啀鍗囩骇
   涓€娆＄洿鍒扮敤灏介搴︺€傚凡鏀逛负璇诲彇 Host slot 鐨勫疄闄呯姸鎬併€傛柊澧炴祴璇曟柇瑷€ epoch 绋冲畾銆?
## 5. 濂戠害鍙樻洿

`display_mode` 涓庢満鍒跺湪鍙湁涓や釜鍙栧€兼椂鏄悓鏋勭殑锛屽洜姝?`GET /api/browser/display-mode`
鍘熷厛浠庡疄鏃?Host 鍙鎬у弽鎺?`display_mode`銆傚姞鍏?`auto` 鍚庤繖鏉℃帹鏂け鏁堬細`auto` 涓?`headless` 閮借〃鐜颁负 headless锛岀瓥鐣ユ棤娉曚粠鏈哄埗杩樺師銆?
```
GET /api/browser/display-mode
{
  "display_mode": "auto",              // 鐢ㄦ埛绛栫暐锛屽彇鑷寔涔呭寲瀛樺偍
  "effective_visibility": "headless"   // 褰撳墠鏈哄埗锛屽彧璇?}

PUT { "display_mode": "auto" }
```

`PUT` 浣跨敤鐙珛璇锋眰绫诲瀷锛屽洜姝?`deny_unknown_fields` 浼?*鎷掔粷**瀹㈡埛绔吉閫犵殑
`effective_visibility`锛岃€屼笉鏄潤榛樺拷鐣ャ€?
**`ui-api-contract-version.txt`锛?9 鈫?20銆?*

鍋忓ソ涓栦唬鏍囪 `BROWSER_DISPLAY_MODE_POLICY_VERSION`锛歚2` 鈫?`3`锛?`agent.browserUse.displayModeVersion` 绫诲瀷鏀惧涓?`2 | 3`锛屽洜涓鸿縼绉讳粛闇€璇诲彇鏃ф爣璁?鏉ュ垽瀹?`external` 鏄惁涓烘槑纭€夋嫨銆?
## 6. 杩佺Щ鐭╅樀

| 瀛橀噺鐘舵€?| 杩佺Щ缁撴灉 | 鐞嗙敱 |
| --- | --- | --- |
| v3 + 鍚堟硶鍊?| 鍘熸牱淇濈暀 | 鏉冨▉ |
| v2 + `external` | `external`锛堥噸鏂扮洊 v3 鏍囪锛?| v2 鏍囪璇佹槑鏄槑纭€夋嫨锛屼笉鑳芥倓鎮勬敹鍥?|
| v2 + `headless` | `auto` | v2 瀵?*鎵€鏈?*瀹夎閮芥寔涔呭寲 `headless`锛屽畠鍙嶆槧鏃ч粯璁よ€岄潪鍐冲畾 |
| 鏃犵増鏈?+ `external` | `auto` | 鍙兘鏄粠 `silent=false` 鎺ㄦ柇鑰屾潵锛?*涓嶅緱**璁╃獥鍙ｅ娲?|
| 鏃犵増鏈?/ 鏇存棫 / legacy-silent | `auto` | 澶辫触鍏抽棴鏂瑰悜锛屼笖 `auto` 浠嶉潤榛樺惎鍔?|
| v3 + 闈炴硶鍊?| `auto` | 淇 |

鍓嶇 `migrateBrowserDisplayMode` 涓庡悗绔?`resolve_browser_display_mode` 瀹炵幇鍚屼竴濂?瑙勫垯锛岄伩鍏嶄袱渚ф紓绉汇€?
## 7. 楠岃瘉

| 妫€鏌?| 缁撴灉 |
| --- | --- |
| `nomifun-browser-platform --lib` | 272 passed / 0 failed |
| `nomi-browser --lib` | 236 passed / 0 failed / 6 ignored |
| `nomifun-app --features browser-use --lib` | 462 passed / 0 failed |
| `bun run test:ui` | 2212 passed / 1 failed锛堣 搂8 涓婃父鏃㈡湁锛?|
| `check:i18n` / `theme` / `icons` / `dead-css` / 涓や釜杈圭晫妫€鏌?/ `agent-vocabulary` | 鍏ㄩ儴閫氳繃 |
| `cargo fmt`锛堟敼鍔ㄥ寘锛夈€乣git diff --check` | 閫氳繃 |

鏈墽琛岋細鐪熷疄 Chrome 鐨?`integration_managed_host --ignored` 楠屾敹闆嗐€?`cargo fmt --all` 鍦ㄦ湰鏈哄洜 Windows 璺緞杩囬暱鎶?os error 206锛屾敼鐢?`-p` 閫愬寘妫€鏌ャ€?
## 8. 涓婃父鏃㈡湁鐮存崯锛堥潪鏈寮曞叆锛?
鍧囧湪 `origin/main` 鐨?`180cabe0` 涓娿€佷笉甯︽湰娆′换浣曟敼鍔ㄥ鐜拌繃锛?
- `bun run typecheck` 鎶?4 涓敊璇紝浣嶄簬 `AboutModalContent.tsx` 涓?  `FeedbackReportModal.tsx`锛屾簮鑷?`9876b2f1 chore: scrub public contact pii`
  鍒犻櫎浜?`email`/`emailHref`/`trailingFallback` 浣嗚皟鐢ㄧ偣浠嶅湪寮曠敤銆傝繖浼氳
  `bun run check` 鍦?typecheck 闃舵鎻愬墠閫€鍑猴紝鍥犳鍚庣画鍚勯」妫€鏌ラ渶鍗曠嫭鎵ц銆?- `CreateStudio form visual design > keeps the dialog and configuration cards
  compact...` 澶辫触锛屾簮鑷?`1c5f214c style(ui): unify modal visual contract`銆?
鏈鏈慨澶嶄笂杩颁袱椤癸紝瀹冧滑灞炰簬寮曞叆瀹冧滑鐨勪笂娓告彁浜ゃ€?
鍙︽湁涓€椤圭幆澧冩€ч棿姝囷細`nomifun-app` 鍏ㄩ噺璺戝伓鍙?*涓€涓?*澶辫触锛屼笖姣忔鎹竴涓笉鍚屾祴璇?锛坄oversized_body_...`銆乣active_owner_bindings_...`锛夛紝澶辫触鐐规槸 loopback
`.send().await.unwrap()` 浼犺緭閿欒鑰岄潪鏂█锛岄殧绂婚噸璺?8/8 涓?3/3 閫氳繃锛涗笌
`docs/handoffs/2026-08-04-browser-use-task-resource-hardening.md` 搂4.5 璁板綍鐨?鐗瑰緛涓€鑷淬€?
## 9. 蹇呴』璇氬疄琛ㄨ揪鐨勪骇鍝佽竟鐣?
鍏变韩 Chromium Host 涓?*鏃犳硶**鍋氬埌绮剧‘鐨勬寜浠诲姟鐗╃悊鍐呭瓨褰掑洜鈥斺€?`shared_rss_estimate_bytes` 鏈川鏄及绠椼€傚洜姝や及绠楀€煎彧搴旂敤浜庨檺娴佷笌闄嶇骇锛?**涓嶅簲**浣滀负寮烘潃鐢ㄦ埛鍓嶅彴宸ヤ綔鐨勫敮涓€渚濇嵁銆傝嫢纭疄闇€瑕佺‖鎬х墿鐞嗛殧绂伙紝鍞竴姝ｇ‘鍋氭硶鏄?姣忎换鍔＄嫭绔?Host + Job Object/cgroup锛屽苟鎺ュ彈鍩虹嚎杩涚▼涓庡唴瀛樼殑澧炲姞銆?
鍚岀悊锛宲er-lane 鐨勫彲瑙佹€у崌绾ф槸涓€娆¤繘绋嬫浛鎹紝涓嶆槸绐楀彛灞炴€у垏鎹紱浠讳綍鎶婂畠鎻忚堪涓?銆屽垏鎹㈢獥鍙ｆ樉绀恒€嶇殑鏂囨閮戒細璇鐢ㄦ埛瀵瑰叾浠ｄ环鐨勯鏈熴€?
## 10. 鍏抽敭浠ｇ爜浣嶇疆

- 褰掑洜搴﹂噺锛歚crates/backend/nomifun-app/src/services.rs`
- 璧勬簮绛栫暐甯搁噺锛歚crates/backend/nomifun-browser-platform/src/resource.rs`
- 鍥炴敹涓庡崌绾э細`crates/backend/nomifun-browser-platform/src/hub.rs`
- 鍐崇瓥鍐呮牳锛堢函鍑芥暟 + 鐪熷€艰〃锛夛細`crates/backend/nomifun-browser-platform/src/model.rs`
- 閿欒鐮侊細`crates/backend/nomifun-browser-platform/src/error.rs`
- 鎰忓浘瑙ｆ瀽涓庤浆鍙戯細`crates/agent/nomi-browser/src/managed.rs`
- 宸ュ叿 schema锛歚crates/agent/nomi-browser/src/tool.rs`
- 绠＄悊 API 涓庣瓥鐣ユ寔涔呭寲锛歚crates/backend/nomifun-app/src/router/browser_management.rs`
- 鍓嶇杩佺Щ涓庤缃細`ui/src/common/browser/browserSettings.ts`銆?  `ui/src/renderer/components/settings/SettingsModal/contents/BrowserUseSettingsContent.tsx`

