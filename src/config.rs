use clap::ArgMatches;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use super::Error;

const CERT_COMMON_NAME: &str = "UNSAFE Proxide Root Certificate";

pub fn run(matches: &ArgMatches) -> Result<(), Error>
{
    match matches.subcommand() {
        ("ca", Some(matches)) => run_ca(matches),
        (cmd, _) => unreachable!("Unknown command: {}", cmd),
    }
}

pub fn run_ca(matches: &ArgMatches) -> Result<(), Error>
{
    // Handle revoke first.
    if matches.is_present("revoke") || matches.is_present("trust") {
        os::revoke_ca(matches)?;
    }

    // If 'revoke' was the only command, we'll interrupt here.
    if !(matches.is_present("create") || matches.is_present("trust")) {
        return Ok(());
    }

    let cert_file = matches
        .value_of("ca-certificate")
        .unwrap_or_else(|| "proxide_ca.crt");
    let key_file = matches
        .value_of("ca-key")
        .unwrap_or_else(|| "proxide_ca.key");

    if matches.is_present("create") {
        create_ca(matches, cert_file, key_file)?;
    }

    // Trust the certificate if the user asked for that.
    if matches.is_present("trust") {
        trust_ca(matches, cert_file, key_file)?;
    }

    Ok(())
}

fn trust_ca(matches: &ArgMatches, cert_file: &str, key_file: &str) -> Result<(), Error>
{
    // Technically if all the user wanted to do was '--create' we wouldn't really need to
    // do this check, but it doesn't really hurt either, unless you count the extra disk
    // access (which I don't!).
    for file in &[cert_file, key_file] {
        if !Path::new(file).is_file() {
            return Err(Error::ArgumentError {
                msg: format!(
                    "Could not open '{}', use --create if you need to create a new CA certificate",
                    file
                ),
            });
        }
    }

    os::trust_ca(cert_file, matches)?;

    Ok(())
}

fn create_ca(matches: &ArgMatches, cert_file: &str, key_file: &str) -> Result<(), Error>
{
    // If the user didn't specify --force we'll need to ensure we are not overwriting
    // any existing files during create.
    if !matches.is_present("force") {
        for file in &[cert_file, key_file] {
            if Path::new(file).is_file() {
                return Err(Error::ArgumentError {
                    msg: format!(
                        "File '{}' already exists. Use --force to overwrite it.",
                        file
                    ),
                });
            }
        }
    }

    // Set up the rcgen certificate parameters for the new certificate.
    //
    // Note that at least on Windows the common name is used to later find and revoke
    // the certificate so it shouldn't be changed without a good reason. If it's
    // changed here, it would be best if new versions of Proxide still supported the
    // old names in the 'revoke' command.
    let mut ca_params = rcgen::CertificateParams::new(vec![]);
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let mut key_usage = rcgen::CustomExtension::from_oid_content(
        &[2, 5, 29, 15],
        vec![
            0x03, // Tag = BIT STRING
            0x02, // Length = 2 bytes
            0x01, // Unused bits = 1
            0x86, // Data; bits FROM LEFT TO RIGHT:
                  // - signature (0th, 0x80),
                  // - sign cert (5th, 0x04),
                  // - sign CRL (6th, 0x02)
        ],
    );
    key_usage.set_criticality(true);
    ca_params.custom_extensions = vec![key_usage];
    ca_params.distinguished_name = rcgen::DistinguishedName::new();
    /*
    ca_params.not_after = chrono::Utc::now()
        + chrono::Duration::days(
            matches
                .value_of("duration")
                .expect("Duration was missing") // Marked as required for clap.
                .parse::<i64>()
                .expect("Duration was invalid") // Validated to be u32 by clap.
                .min(/* 365.2425 * 2000 = */ 730485), // ASN has trouble with giant dates so max out at 2000 years.
        );
        */
    ca_params
        .distinguished_name
        .push(rcgen::DnType::OrganizationName, "UNSAFE");
    ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "UNSAFE Proxide Root Certificate"); // See the comment above.
    let ca_cert = rcgen::Certificate::from_params(ca_params).unwrap();

    File::create(cert_file)
        .map_err(|_| Error::ArgumentError {
            msg: format!(
                "Could not open the certificate file '{}' for writing",
                cert_file
            ),
        })?
        .write_all(
            ca_cert
                .serialize_pem()
                .map_err(|e| Error::RuntimeError { msg: e.to_string() })?
                .as_bytes(),
        )
        .map_err(|_| Error::ArgumentError {
            msg: format!("Could not write certificate to '{}'", cert_file),
        })?;
    File::create(key_file)
        .map_err(|_| Error::ArgumentError {
            msg: format!(
                "Could not open the private key file '{}' for writing",
                key_file
            ),
        })?
        .write_all(ca_cert.serialize_private_key_pem().as_bytes())
        .map_err(|_| Error::ArgumentError {
            msg: format!("Could not write private key to '{}'", key_file),
        })?;

    Ok(())
}

#[cfg(not(windows))]
mod os
{
    use super::*;
    pub fn revoke_ca(_matches: &ArgMatches) -> Result<(), Error>
    {
        return Err(Error::RuntimeError {
            msg: "--revoke is not supported on this platform".to_string(),
        });
    }

    pub fn trust_ca(_cert_file: &str, _matches: &ArgMatches) -> Result<(), Error>
    {
        return Err(Error::RuntimeError {
            msg: "--trust is not supported on this platform".to_string(),
        });
    }
}

#[cfg(windows)]
mod os
{
    use super::*;
    pub fn revoke_ca(matches: &ArgMatches) -> Result<(), Error>
    {
        // The revoke may happen either explicitly through --revoke or implicitly through --trust.
        // Since the use of these two options conflict with each other, we need to only care about
        // one of them here.
        let store = matches
            .value_of("revoke")
            .or_else(|| matches.value_of("trust"))
            .unwrap_or("user");
        match store {
            "user" | "system" | "all" => (),
            v => {
                return Err(Error::ArgumentError {
                    msg: format!("Invalid certificate store '{}'", v),
                })
            }
        }

        if store == "all" || store == "system" {
            println!("Removing the previous Proxide CA certificates from the System store");
            std::process::Command::new("certutil")
                .arg("-delstore")
                .arg("Root")
                .arg(CERT_COMMON_NAME)
                .spawn()
                .and_then(|mut process| process.wait())
                .map(|_| ())
                .map_err(|e| Error::RuntimeError {
                    msg: format!("Failed to revoke the certificates with certutil: {}", e),
                })?;
        }

        if store == "all" || store == "user" {
            println!("Removing the previous Proxide CA certificates from the User store");
            std::process::Command::new("certutil")
                .arg("-delstore")
                .arg("-user")
                .arg("Root")
                .arg(CERT_COMMON_NAME)
                .spawn()
                .and_then(|mut process| process.wait())
                .map(|_| ())
                .map_err(|e| Error::RuntimeError {
                    msg: format!("Failed to revoke the certificates with certutil: {}", e),
                })?;
        }

        Ok(())
    }

    pub fn trust_ca(cert_file: &str, matches: &ArgMatches) -> Result<(), Error>
    {
        let trust = matches.value_of("trust").unwrap_or("user");
        match trust {
            "user" | "system" | "all" => (),
            v => {
                return Err(Error::ArgumentError {
                    msg: format!("Invalid --trust value '{}'", v),
                })
            }
        }

        if trust == "all" || trust == "system" {
            println!("Add system");
            std::process::Command::new("certutil")
                .arg("-addstore")
                .arg("-v")
                .arg("Root")
                .arg(cert_file)
                .spawn()
                .and_then(|mut process| process.wait())
                .map_err(|e| Error::RuntimeError {
                    msg: format!("Failed to import the certificate with certutil: {}", e),
                })?;
        }

        if trust == "all" || trust == "user" {
            println!("Add user");
            std::process::Command::new("certutil")
                .arg("-addstore")
                .arg("-user")
                .arg("Root")
                .arg(cert_file)
                .spawn()
                .and_then(|mut process| process.wait())
                .map_err(|e| Error::RuntimeError {
                    msg: format!("Failed to import the certificate with certutil: {}", e),
                })?;
        }

        Ok(())
    }
}
