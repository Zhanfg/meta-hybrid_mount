// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;

use crate::conf::config::Config;
#[cfg(feature = "kasumi")]
use crate::sys::kasumi;

#[derive(Debug, Clone, Default)]
pub struct BackendCapabilities {
    kasumi_status: String,
    kasumi_usable: bool,
}

impl BackendCapabilities {
    pub fn detect(config: &Config) -> Result<Self> {
        #[cfg(not(feature = "kasumi"))]
        {
            let _ = config;
            Ok(Self::kasumi_disabled())
        }

        #[cfg(feature = "kasumi")]
        {
            if !config.kasumi.enabled {
                return Ok(Self::kasumi_disabled());
            }

            match kasumi::check_status() {
                Ok(status) => Ok(Self {
                    kasumi_status: kasumi::status_name(status).to_string(),
                    kasumi_usable: matches!(status, kasumi::KasumiStatus::Available),
                }),
                Err(error) => {
                    crate::scoped_log!(
                        warn,
                        "backend_capabilities",
                        "kasumi probe failed; backend marked unavailable: error={:#}",
                        error
                    );
                    Ok(Self {
                        kasumi_status: "probe_error".to_string(),
                        kasumi_usable: false,
                    })
                }
            }
        }
    }

    fn kasumi_disabled() -> Self {
        Self {
            kasumi_status: "disabled".to_string(),
            kasumi_usable: false,
        }
    }

    pub fn can_use_kasumi(&self) -> bool {
        self.kasumi_usable
    }

    pub fn kasumi_status(&self) -> &str {
        &self.kasumi_status
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_kasumi_does_not_require_a_kernel_probe() {
        let mut config = Config::default();
        config.kasumi.enabled = false;

        let capabilities = BackendCapabilities::detect(&config).unwrap();
        assert!(!capabilities.can_use_kasumi());
        assert_eq!(capabilities.kasumi_status(), "disabled");
    }
}
