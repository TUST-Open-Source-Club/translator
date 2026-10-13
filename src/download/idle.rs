//! Bound an individual network read without limiting a multi-gigabyte transfer.
//! ureq 3.4.2's recv_body timeout covers the entire body, not each read.
use std::time::Duration;
use ureq::unversioned::transport::{Buffers, ConnectionDetails, Connector, NextTimeout, Transport};

#[derive(Debug)]
pub(super) struct IdleConnector(pub Duration);

impl<T: Transport> Connector<T> for IdleConnector {
    type Out = IdleTransport<T>;
    fn connect(
        &self,
        _: &ConnectionDetails,
        transport: Option<T>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        Ok(transport.map(|inner| IdleTransport {
            inner,
            idle: self.0,
        }))
    }
}

#[derive(Debug)]
pub(super) struct IdleTransport<T> {
    inner: T,
    idle: Duration,
}

impl<T: Transport> Transport for IdleTransport<T> {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }
    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.inner.transmit_output(amount, timeout)
    }
    fn await_input(&mut self, mut timeout: NextTimeout) -> Result<bool, ureq::Error> {
        if timeout.after > self.idle.into() {
            timeout.after = self.idle.into();
            timeout.reason = ureq::Timeout::RecvBody;
        }
        self.inner.await_input(timeout)
    }
    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }
    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}
