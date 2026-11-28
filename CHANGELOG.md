# Changelog

## [0.6.0](https://github.com/scarletkc/colab-playground/compare/v0.5.0...v0.6.0) (2026-11-28)


### Added

* **app:** add a GitHub Sponsors link to Settings → About ([#116](https://github.com/scarletkc/colab-playground/issues/116)) ([09d4a4c](https://github.com/scarletkc/colab-playground/commit/09d4a4c21856e0958f82da2a3439073c80f592ee))
* **app:** choose which plugins the Overview shows, installed ones included ([#122](https://github.com/scarletkc/colab-playground/issues/122)) ([d4297de](https://github.com/scarletkc/colab-playground/commit/d4297de4c38d657ca76298ec730f61a49d277368))
* **app:** extend auto-stops and forward ports from the command line ([#117](https://github.com/scarletkc/colab-playground/issues/117)) ([208ddb5](https://github.com/scarletkc/colab-playground/commit/208ddb54f9c50d8ee69cfcfe9b831206c3e5a336))
* **app:** read the running app's accounts, runtimes and usage from the command line ([#108](https://github.com/scarletkc/colab-playground/issues/108)) ([c18197b](https://github.com/scarletkc/colab-playground/commit/c18197b31400639084f79c5593d333758f0abd2b))
* **app:** start and stop runtimes from the command line ([#115](https://github.com/scarletkc/colab-playground/issues/115)) ([0401608](https://github.com/scarletkc/colab-playground/commit/0401608e3e1b428b1e9cb4f87bba29f1d5c7d6cd))
* show the country and region each runtime runs in ([#104](https://github.com/scarletkc/colab-playground/issues/104)) ([3a13e56](https://github.com/scarletkc/colab-playground/commit/3a13e5646bd3e4d5b6548731518cdccb6bbe12d8))


### Changed

* **app:** bring the running window forward instead of starting a second copy ([c18197b](https://github.com/scarletkc/colab-playground/commit/c18197b31400639084f79c5593d333758f0abd2b))
* **app:** check for updates at every start ([#107](https://github.com/scarletkc/colab-playground/issues/107)) ([35ebe12](https://github.com/scarletkc/colab-playground/commit/35ebe125476f8c7c0770f5f5c432004a65b89601))
* **app:** open forms for a problem or an idea from "Give feedback" ([#114](https://github.com/scarletkc/colab-playground/issues/114)) ([9cd914e](https://github.com/scarletkc/colab-playground/commit/9cd914e7c527b2a3159f376c7b956d6f2f2825c5))


### Removed

* drop demo data from release downloads ([#109](https://github.com/scarletkc/colab-playground/issues/109)) ([fce4a4b](https://github.com/scarletkc/colab-playground/commit/fce4a4bf42cf765bd82b46ca37391aa4ce2826d9))


### Fixed

* **app:** show plugins that fail to load and what changed in the plugins folder without a restart ([#121](https://github.com/scarletkc/colab-playground/issues/121)) ([190e341](https://github.com/scarletkc/colab-playground/commit/190e341597738aea181f71efaba4edf0a966c077))
* **app:** stop two copies of the app from undoing each other's settings ([#112](https://github.com/scarletkc/colab-playground/issues/112)) ([38cb95c](https://github.com/scarletkc/colab-playground/commit/38cb95c27200bcc826abff018bac72f8a69402b7))
* **core:** keep a port forward opened during a stop from starting a new runtime ([#105](https://github.com/scarletkc/colab-playground/issues/105)) ([a3af373](https://github.com/scarletkc/colab-playground/commit/a3af373717e4e8e0c71acc20ad720857bee79254))
* **core:** keep a shell notice from hiding why a port forward failed on macOS ([#110](https://github.com/scarletkc/colab-playground/issues/110)) ([ffc16b1](https://github.com/scarletkc/colab-playground/commit/ffc16b1b278f6394a3ca07b54145d611af56ccbd))
* **core:** keep port forwards working when a path contains % ([00b4c4d](https://github.com/scarletkc/colab-playground/commit/00b4c4d8f3ce5107c35318786854614afd51d3c3))
* **core:** never let a port forward's ProxyCommand create a runtime ([#106](https://github.com/scarletkc/colab-playground/issues/106)) ([6bd1306](https://github.com/scarletkc/colab-playground/commit/6bd130692b38968ab6c4e2c15d06f1b6c5c5d7e1))

## [0.5.0](https://github.com/scarletkc/colab-playground/compare/v0.4.0...v0.5.0) (2026-11-26)


### Added

* **app:** show what the chosen hardware burns per hour when starting a session ([#73](https://github.com/scarletkc/colab-playground/issues/73)) ([3b5bcd3](https://github.com/scarletkc/colab-playground/commit/3b5bcd3028793e5b9d5886823c8c31d804d593c5))


### Changed

* **core:** write the session ledger once per usage round ([#76](https://github.com/scarletkc/colab-playground/issues/76)) ([146ce3a](https://github.com/scarletkc/colab-playground/commit/146ce3a301d907044dac3cdc7221ff60c3b3e4ae))


### Fixed

* **app:** name a new session after the hardware chosen in the form, not the default ([#74](https://github.com/scarletkc/colab-playground/issues/74)) ([bb17287](https://github.com/scarletkc/colab-playground/commit/bb17287ddc27a621f165b5f42a970002613c50e8))
* **app:** point to Settings when colab is still not found after installing the CLI ([fa25d8c](https://github.com/scarletkc/colab-playground/commit/fa25d8c18eb0db2d36a2faadfd7cbb32f44e809e))
* **core:** find a colab installed by uv when bash has no profile for it ([#69](https://github.com/scarletkc/colab-playground/issues/69)) ([ac9a0a1](https://github.com/scarletkc/colab-playground/commit/ac9a0a138c9a2a259a9b6966d62b6c483bab9683))
* **core:** keep bash's job report out of logs and error details ([#89](https://github.com/scarletkc/colab-playground/issues/89)) ([373f565](https://github.com/scarletkc/colab-playground/commit/373f56594a3f895308bf568e3eb2d58df24cf2a1))
* **core:** keep the llama.cpp outputs off other plugins' demo runs ([#91](https://github.com/scarletkc/colab-playground/issues/91)) ([94f87de](https://github.com/scarletkc/colab-playground/commit/94f87de35c1cf87249adff768b85183ee4a4f9ea))
* **core:** show bash's error messages without a kilobyte of base64 in front ([#90](https://github.com/scarletkc/colab-playground/issues/90)) ([c0d3f4b](https://github.com/scarletkc/colab-playground/commit/c0d3f4b8de06680e0861f8e1ac7993f288f82a43))
* **core:** take a session's burn rate from the account once it runs alone, not its even share ([#82](https://github.com/scarletkc/colab-playground/issues/82)) ([ce740d7](https://github.com/scarletkc/colab-playground/commit/ce740d761f8791b4d14349ff65b9dffc1725aff3))
* **core:** take a session's burn rate from what the account burns beyond the sessions already measured ([#88](https://github.com/scarletkc/colab-playground/issues/88)) ([e4dd1c6](https://github.com/scarletkc/colab-playground/commit/e4dd1c6c71ff974bd8ba370cf70ca6148f2b3592))

## [0.4.0](https://github.com/scarletkc/colab-playground/compare/v0.3.0...v0.4.0) (2026-11-20)


### Added

* **app:** ask for a GitHub star or feedback once the app has proved useful ([#37](https://github.com/scarletkc/colab-playground/issues/37)) ([77e4f37](https://github.com/scarletkc/colab-playground/commit/77e4f37c614593ab7ea60165894fa4a5e21aff83))


### Fixed

* **app:** ask before quitting when the window is closed while the list of runtimes is refreshing ([#47](https://github.com/scarletkc/colab-playground/issues/47)) ([2739f98](https://github.com/scarletkc/colab-playground/commit/2739f98faaa89bdba0d8b2ee1006d5939c2e6e8f))
* **app:** keep a plugin's secret inputs in the form when Deploy is rejected ([#58](https://github.com/scarletkc/colab-playground/issues/58)) ([9994db1](https://github.com/scarletkc/colab-playground/commit/9994db1c1a2100acad4fa9248f27057378fc9eb1))
* **app:** keep port forwards open when an account switch cannot start, stop a plugin check when its dialog closes, and smaller fixes from the audit ([#61](https://github.com/scarletkc/colab-playground/issues/61)) ([3a1a6a1](https://github.com/scarletkc/colab-playground/commit/3a1a6a1914542b16fb76910efd63af4e356fbc87))
* **app:** keep the list of runtimes from showing a stopped runtime, or missing a new one, after a refresh that began earlier ([#48](https://github.com/scarletkc/colab-playground/issues/48)) ([9e3b459](https://github.com/scarletkc/colab-playground/commit/9e3b4593cd02199dbb65f283313ca7f88744d8bb))
* **app:** keep the newest 2,000 lines of a plugin run's log so a chatty script does not slow the window down ([#62](https://github.com/scarletkc/colab-playground/issues/62)) ([ccd8956](https://github.com/scarletkc/colab-playground/commit/ccd8956fa1d270ae65cd0f2ca4910b42dccf5cbf))
* **core:** keep a free port from being reported as in use when a forward starts or reconnects ([#65](https://github.com/scarletkc/colab-playground/issues/65)) ([8a6c51e](https://github.com/scarletkc/colab-playground/commit/8a6c51ea3d644a71be8e12618b4308b256596c24))
* **core:** keep a plugin run's log from falling behind a script that writes a lot, and show its last lines when the run ends ([#57](https://github.com/scarletkc/colab-playground/issues/57)) ([e319e34](https://github.com/scarletkc/colab-playground/commit/e319e342c4c72433e383e3ec4a5969aeafc0cb62))
* **core:** let the spend cap stop a session whose burn rate showed late, and keep a usage poll from switching the account back ([#50](https://github.com/scarletkc/colab-playground/issues/50)) ([aa5d282](https://github.com/scarletkc/colab-playground/commit/aa5d282715ce05217e6c6b9a6f0bc6fe989ea132))
* **core:** list a model's GGUF files when a file name has non-ASCII characters ([#53](https://github.com/scarletkc/colab-playground/issues/53)) ([687f85b](https://github.com/scarletkc/colab-playground/commit/687f85ba0f84ff5d9fd7841e9c0e188c91958bf7))
* **core:** remove the copy staged for a plugin preview when the install dialog closes or the install fails, and a Git checkout that did not finish ([#55](https://github.com/scarletkc/colab-playground/issues/55)) ([12db790](https://github.com/scarletkc/colab-playground/commit/12db790b443138cdffc7d5a7a5ecb3bcd68d093d))
* **core:** stop retrying a port forward that never connects, end ssh when the app is killed, and catch a local port taken on IPv6 ([#42](https://github.com/scarletkc/colab-playground/issues/42)) ([805cb38](https://github.com/scarletkc/colab-playground/commit/805cb3875c4c59b46261d56835ba54b1c724adde))
* keep a second Extend from being lost when the list of runtimes has not refreshed yet ([#54](https://github.com/scarletkc/colab-playground/issues/54)) ([b8ae68e](https://github.com/scarletkc/colab-playground/commit/b8ae68e9cfde704e7809f3d8eebf47ee6534f69c))
* use the Colab CLI path from Settings, retry a failed update check after an hour and fetch usage once per round ([#44](https://github.com/scarletkc/colab-playground/issues/44)) ([9b57ce8](https://github.com/scarletkc/colab-playground/commit/9b57ce8f2a5436640de98d4bc39bc7d6e07d32f4))

## [0.3.0](https://github.com/scarletkc/colab-playground/compare/v0.2.0...v0.3.0) (2026-11-19)


### Added

* download, check and install updates from inside the app ([#41](https://github.com/scarletkc/colab-playground/issues/41)) ([cfd98c2](https://github.com/scarletkc/colab-playground/commit/cfd98c2e6781e97a42cca35fa976d30a27f51a70))


### Changed

* name the downloads without the version ([cfd98c2](https://github.com/scarletkc/colab-playground/commit/cfd98c2e6781e97a42cca35fa976d30a27f51a70))


### Fixed

* **core:** end a session whose runtime was never recorded once nothing running could be it ([#38](https://github.com/scarletkc/colab-playground/issues/38)) ([9e5eb7e](https://github.com/scarletkc/colab-playground/commit/9e5eb7e34b7b1f8eb4800e66a1cf6243bf0ea6f7))

## [0.2.0](https://github.com/scarletkc/colab-playground/compare/v0.1.0...v0.2.0) (2026-11-18)


### Added

* **app:** add Japanese and Korean as display languages ([#19](https://github.com/scarletkc/colab-playground/issues/19)) ([def0fa8](https://github.com/scarletkc/colab-playground/commit/def0fa8049c636d51400b47e9a421c785c451a60))
* **app:** say how long Colab has not answered a request for a runtime ([#24](https://github.com/scarletkc/colab-playground/issues/24)) ([0f6c5f0](https://github.com/scarletkc/colab-playground/commit/0f6c5f029fb7d2f39e0433379d1db9987cbdd4f4))


### Changed

* **app:** show no app mark or name beside the traffic lights on macOS ([#11](https://github.com/scarletkc/colab-playground/issues/11)) ([2583758](https://github.com/scarletkc/colab-playground/commit/2583758f3934a00ad5d58ed42e4c00aa8664c0f4))


### Fixed

* **app:** give the New session and Forward dialogs a scrollbar and the window's height ([#18](https://github.com/scarletkc/colab-playground/issues/18)) ([72e6009](https://github.com/scarletkc/colab-playground/commit/72e60097f291a0a08be7735c86dfee6894c494e6))
* **app:** keep the billing title when only some accounts have unknown runtime ownership ([#31](https://github.com/scarletkc/colab-playground/issues/31)) ([17a099d](https://github.com/scarletkc/colab-playground/commit/17a099d1f7555b6e3519e511aabc2034b3350e19))
* **app:** list plugins before the Colab CLI is installed ([#10](https://github.com/scarletkc/colab-playground/issues/10)) ([778c5f9](https://github.com/scarletkc/colab-playground/commit/778c5f94caba6602023a876316ee5684f073472c))
* **app:** mark the unmanaged-runtime warning as a last reading when usage is stale ([#26](https://github.com/scarletkc/colab-playground/issues/26)) ([bc23ecd](https://github.com/scarletkc/colab-playground/commit/bc23ecd6e11ce265ecad2ee62b91ca0857deb3a9))
* **app:** say when the runtimes left behind by an account switch are unknown ([#20](https://github.com/scarletkc/colab-playground/issues/20)) ([2cdc26c](https://github.com/scarletkc/colab-playground/commit/2cdc26c8cfebc7391de54cfe38ed65b4ac3ccf7a))
* **core:** keep WSL and its session bus up so the keyring is reachable and stays unlocked ([#16](https://github.com/scarletkc/colab-playground/issues/16)) ([d7a0386](https://github.com/scarletkc/colab-playground/commit/d7a0386e80a913feb6c0a4baa87a2e4c6a187caf))
* **core:** let Cancel and Stop all end a runtime request Colab never answers ([#21](https://github.com/scarletkc/colab-playground/issues/21)) ([586e26a](https://github.com/scarletkc/colab-playground/commit/586e26a66cd57e6d843027b99b20b63a83c1c5db))
* **core:** reconcile ended sessions before counting unmanaged runtimes ([#27](https://github.com/scarletkc/colab-playground/issues/27)) ([7048695](https://github.com/scarletkc/colab-playground/commit/704869572c0f9249897eabb613e5f79796c2640a))
* **core:** show wsl.exe messages legibly and stop treating them as a WSL distribution ([#22](https://github.com/scarletkc/colab-playground/issues/22)) ([91b139a](https://github.com/scarletkc/colab-playground/commit/91b139ae092846d00a9364dc27fa1c644904598e)), closes [#14](https://github.com/scarletkc/colab-playground/issues/14)

## 0.1.0 (2026-11-17)


### Added

* initial release ([6701a59](https://github.com/scarletkc/colab-playground/commit/6701a59889884510a97f4e17ae7b03a5416955f5))
