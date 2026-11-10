use std::{io, sync::Arc};

use super::{Cancellation, ControlDevice, validate_timeout};
use crate::endpoint::{EndpointReadPlan, validate_read_parameters};

/// A Bulk or Interrupt IN read bound to one interface claim.
#[derive(Debug)]
pub struct PreparedEndpointRead {
    pub(super) plan: EndpointReadPlan,
    pub(super) timeout_ms: u32,
    claim: Arc<()>,
}

impl PreparedEndpointRead {
    /// Returns the inventory metadata for this request.
    #[must_use]
    pub fn plan(&self) -> EndpointReadPlan {
        self.plan
    }
}

impl ControlDevice {
    /// Prepares a Bulk or Interrupt IN read after an explicit claim and seal.
    ///
    /// The request expires when its interface is released or the connection
    /// closes. The timeout is in milliseconds and applies to the data transfer.
    ///
    /// # Errors
    ///
    /// Returns `PermissionDenied` without a sealed handle whose only claim
    /// is for the requested interface, or `InvalidInput` for an invalid endpoint, length or timeout.
    pub fn prepare_endpoint_read(
        &self,
        plan: EndpointReadPlan,
        timeout_ms: u32,
    ) -> io::Result<PreparedEndpointRead> {
        self.require_endpoint_handle()?;
        self.prepare_read_on_claim(plan, timeout_ms)
    }

    /// Revalidates the active endpoint and submits the prepared IN transfer.
    ///
    /// Short and empty responses are valid. Cancellation skips a pending
    /// transfer or discards its result after a running ioctl returns.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale claim, changed endpoint metadata, OS
    /// failure, cancellation, or a response larger than the requested buffer.
    pub fn run_endpoint_read(
        &mut self,
        request: &PreparedEndpointRead,
        cancellation: &Cancellation,
    ) -> io::Result<Vec<u8>> {
        self.require_endpoint_handle()?;
        self.validate_read_claim(request)?;
        cancellation.check()?;
        self.backend.validate_endpoint(request.plan)?;
        let mut bytes = vec![0; request.plan.length];
        cancellation.check()?;
        let result = self
            .backend
            .submit_endpoint(request.plan, request.timeout_ms, &mut bytes);
        cancellation.check()?;
        let actual = result?;
        if actual > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "USB endpoint read exceeded the requested length",
            ));
        }
        bytes.truncate(actual);
        Ok(bytes)
    }

    fn require_endpoint_handle(&self) -> io::Result<()> {
        if self.claims_sealed && self.claimed.len() == 1 {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "endpoint reads require a sealed handle with exactly one interface claim",
            ))
        }
    }

    pub(super) fn prepare_read_on_claim(
        &self,
        plan: EndpointReadPlan,
        timeout_ms: u32,
    ) -> io::Result<PreparedEndpointRead> {
        validate_read_parameters(plan.endpoint, plan.length)?;
        validate_timeout(timeout_ms)?;
        let claim = self.claimed.get(&plan.interface).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "claim the endpoint's interface before reading",
            )
        })?;
        Ok(PreparedEndpointRead {
            plan,
            timeout_ms,
            claim: Arc::clone(claim),
        })
    }

    pub(super) fn validate_read_claim(&self, request: &PreparedEndpointRead) -> io::Result<()> {
        if self
            .claimed
            .get(&request.plan.interface)
            .is_some_and(|claim| Arc::ptr_eq(claim, &request.claim))
        {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "endpoint read no longer belongs to the active interface claim",
            ))
        }
    }
}
