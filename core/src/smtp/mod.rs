// check-if-email-exists
// Copyright (C) 2018-2022 Reacher

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.

// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

mod connect;
mod error;
mod gmail;
mod http_api;
mod outlook;
mod parser;
mod yahoo;

use std::default::Default;

use async_smtp::EmailAddress;
use serde::{Deserialize, Serialize};
use trust_dns_proto::rr::Name;

use crate::{util::input_output::CheckEmailInput, LOG_TARGET};
use connect::check_smtp_with_retry;
pub use error::*;

use self::{
	gmail::is_gmail,
	outlook::{is_microsoft365, is_outlook},
	yahoo::is_yahoo,
};

/// Details that we gathered from connecting to this email via SMTP
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct SmtpDetails {
	/// Are we able to connect to the SMTP server?
	pub can_connect_smtp: bool,
	/// Is this email account's inbox full?
	pub has_full_inbox: bool,
	/// Does this domain have a catch-all email address?
	pub is_catch_all: bool,
	/// Can we send an email to this address?
	pub is_deliverable: bool,
	/// Is the email blocked or disabled by the provider?
	pub is_disabled: bool,
}

/// Get all email details we can from one single `EmailAddress`, without
/// retries.
pub async fn check_smtp(
	to_email: &EmailAddress,
	host: &Name,
	port: u16,
	domain: &str,
	input: &CheckEmailInput,
) -> Result<SmtpDetails, SmtpError> {
	let host = host.to_string();

	if input.skipped_domains.iter().any(|d| host.contains(d)) {
		return Err(SmtpError::SkippedDomain(format!(
			"Reacher currently cannot verify emails from @{domain}"
		)));
	}

	if input.yahoo_use_api && is_yahoo(&host) {
		return yahoo::check_yahoo(to_email, input)
			.await
			.map_err(|err| err.into());
	}
	if input.gmail_use_api && is_gmail(&host) {
		return gmail::check_gmail(to_email, input)
			.await
			.map_err(|err| err.into());
	}
	if input.microsoft365_use_api && is_microsoft365(&host) {
		match outlook::microsoft365::check_microsoft365_api(to_email, input).await {
			Ok(Some(smtp_details)) => return Ok(smtp_details),
			// Continue in the event of an error/ambiguous result.
			Err(err) => {
				log::debug!(
					target: LOG_TARGET,
					"[email={}] microsoft365 error: {:?}",
					to_email,
					err,
				);
			}
			_ => {}
		}
	}
	#[cfg(feature = "headless")]
	if let Some(webdriver) = &input.hotmail_use_headless {
		if is_outlook(&host) {
			return outlook::hotmail::check_password_recovery(to_email, webdriver)
				.await
				.map_err(|err| err.into());
		}
	}

	check_smtp_with_retry(to_email, &host, port, domain, input, input.retries).await
}

#[cfg(test)]
mod tests {
	use super::{check_smtp, CheckEmailInput, SmtpError};
	use async_smtp::{smtp::error::Error, EmailAddress};
	use std::{str::FromStr, time::Duration};
	use tokio::runtime::Runtime;
	use trust_dns_proto::rr::Name;

	#[test]
	fn should_timeout() {
		let runtime = Runtime::new().unwrap();

		let to_email = EmailAddress::from_str("foo@gmail.com").unwrap();
		let host = Name::from_str("alt4.aspmx.l.google.com.").unwrap();
		let mut input = CheckEmailInput::default();
		input.set_smtp_timeout(Some(Duration::from_millis(1)));

		let res = runtime.block_on(check_smtp(&to_email, &host, 25, "gmail.com", &input));
		match res {
			Err(SmtpError::SmtpError(Error::Io(_))) => (), // ErrorKind == Timeout
			_ => panic!("check_smtp did not time out"),
		}
	}

	#[test]
	fn should_skip_domains() {
		let runtime = Runtime::new().unwrap();

		let to_email = EmailAddress::from_str("foo@icloud.com").unwrap();
		let host = Name::from_str("mx01.mail.icloud.com.").unwrap();
		let mut input = CheckEmailInput::default();
		input.set_skipped_domains(vec![".mail.icloud.com.".into()]);

		let res = runtime.block_on(check_smtp(&to_email, &host, 25, "icloud.com", &input));
		match res {
			Err(SmtpError::SkippedDomain(_)) => (),
			r => panic!("{:?}", r),
		}
	}
}
