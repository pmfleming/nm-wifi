use anyhow::{Context, Result};

use super::{NM_IFACE, NM_PATH, Nm};
use crate::model::ConnectivityStatus;

impl Nm {
    pub(crate) fn connectivity_check(&self) -> Result<ConnectivityStatus> {
        let nm = self.proxy(NM_PATH, NM_IFACE)?;
        let code: u32 = nm
            .call("CheckConnectivity", &())
            .context("CheckConnectivity")?;
        Ok(ConnectivityStatus::from_nm_code(code))
    }
}
