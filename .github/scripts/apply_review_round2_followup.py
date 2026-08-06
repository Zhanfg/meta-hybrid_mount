from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(
            f"{path}: expected one match, found {count}: {old[:120]!r}"
        )
    path.write_text(text.replace(old, new, 1))


controller = Path("src/core/controller.rs")
replace_once(
    controller,
    '''        let mut plan = prepare::prepare_mount_plan(
            modules,
            self.state.handle.mount_point(),
            &self.backend_capabilities,
        )?;
''',
    '''        let plan = prepare::prepare_mount_plan(
            modules,
            self.state.handle.mount_point(),
            &self.backend_capabilities,
        )?;
        #[cfg(feature = "kasumi")]
        let mut plan = plan;
''',
)

startup = Path("src/core/startup/mod.rs")
replace_once(
    startup,
    '''    let mut config = match load_config() {
        Ok(config) => config,
        Err(error) => {
            crate::scoped_log!(error, "startup", "config load failed: error={:#}", error);
            return Err(error);
        }
    };
''',
    '''    let config = match load_config() {
        Ok(config) => config,
        Err(error) => {
            crate::scoped_log!(error, "startup", "config load failed: error={:#}", error);
            return Err(error);
        }
    };
    #[cfg(feature = "kasumi")]
    let mut config = config;
''',
)

api_mock = Path("webui/src/lib/api.mock.ts")
replace_once(
    api_mock,
    '''  return {
    loaded: kasumi.lkmLoaded,
    module_name: "kasumi_lkm",
''',
    '''  return {
    loaded: kasumi.lkmLoaded,
    managed: true,
    module_name: "kasumi_lkm",
''',
)
