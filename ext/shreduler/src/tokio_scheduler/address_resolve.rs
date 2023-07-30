use crate::new_base_error;

use super::prelude::*;

impl TokioScheduler {
    /// Resolves a hostname to a list of IP addresses, returning `None` if the
    /// hostname cannot be resolved.
    ///
    /// Returns a Future that resolves to a list of IP addresses.
    #[tracing::instrument]
    pub fn address_resolve(&self, hostname: RString) -> Result<Value, Error> {
        let hostname = hostname.to_string()?;

        let future = async move {
            // See https://github.com/socketry/async/issues/180 for more details.
            let hostname = hostname.split('%').next().unwrap_or(&hostname);
            let mut split = hostname.splitn(2, ':');

            let Some(host) = split.next() else {
                return Ok(RArray::new().into_value());
            };

            // Match the behavior of MRI, which returns an empty array if the port is given
            if split.next().is_some() {
                return Ok(RArray::new().into_value());
            }

            let host_lookup = tokio::net::lookup_host((host, 80)).await;
            let host_lookup =
                host_lookup.map_err(|e| new_base_error!("Could not resolve hostname: {}", e))?;
            let addresses = RArray::new();

            for address in host_lookup {
                addresses.push(address.ip().to_string())?;
            }

            debug!(?addresses, "returning addresses");
            Ok(*addresses)
        };

        self.spawn_and_transfer(future)
    }
}
