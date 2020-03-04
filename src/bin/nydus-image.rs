// Copyright 2020 Ant Financial. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use image_builder::builder;

#[macro_use(crate_version, crate_authors)]
extern crate clap;
use clap::{App, Arg};
use uuid::Uuid;

use std::fs::File;
use std::io::Result;

use rafs::storage::oss_backend::OSS;

fn main() -> Result<()> {
    let cmd_arguments = App::new("nydus image builder")
        .version(crate_version!())
        .author(crate_authors!())
        .about("Build image using nydus format.")
        .arg(
            Arg::with_name("SOURCE")
                .long("source")
                .help("source directory")
                .required(true)
                .index(1),
        )
        .arg(
            Arg::with_name("blob")
                .long("blob")
                .help("blob file path")
                .takes_value(true)
                .min_values(1),
        )
        .arg(
            Arg::with_name("blod_id")
                .long("blod_id")
                .help("blob id")
                .takes_value(true)
                .min_values(0),
        )
        .arg(
            Arg::with_name("bootstrap")
                .long("bootstrap")
                .help("bootstrap file path")
                .takes_value(true)
                .min_values(1),
        )
        .arg(
            Arg::with_name("oss_endpoint")
                .long("oss_endpoint")
                .help("oss endpoint")
                .takes_value(true)
                .min_values(0),
        )
        .arg(
            Arg::with_name("oss_access_key_id")
                .long("oss_access_key_id")
                .help("oss access key id")
                .takes_value(true)
                .min_values(0),
        )
        .arg(
            Arg::with_name("oss_access_key_secret")
                .long("oss_access_key_secret")
                .help("oss access key secret")
                .takes_value(true)
                .min_values(0),
        )
        .arg(
            Arg::with_name("oss_bucket_name")
                .long("oss_bucket_name")
                .help("oss bucket name")
                .takes_value(true)
                .min_values(0),
        )
        .get_matches();

    let source_path = cmd_arguments.value_of("SOURCE").unwrap();
    let blob_path = cmd_arguments.value_of("blob").unwrap();
    let bootstrap_path = cmd_arguments.value_of("bootstrap").unwrap();

    let mut blob_id = Uuid::new_v4().to_string();
    if let Some(p_blob_id) = cmd_arguments.value_of("blob_id") {
        blob_id = String::from(p_blob_id);
    }

    let mut ib = builder::Builder::new(source_path, blob_path, bootstrap_path, blob_id.as_str())?;
    ib.build()?;

    if let Some(oss_endpoint) = cmd_arguments.value_of("oss_endpoint") {
        let oss_access_key_id = cmd_arguments.value_of("oss_access_key_id").unwrap();
        let oss_access_key_secret = cmd_arguments.value_of("oss_access_key_secret").unwrap();
        let oss_bucket_name = cmd_arguments.value_of("oss_bucket_name").unwrap();

        let oss = OSS::new(
            oss_endpoint,
            oss_access_key_id,
            oss_access_key_secret,
            oss_bucket_name,
        );

        let blob_file = File::open(blob_path)?;
        oss.put_object(blob_id.as_str(), blob_file)?;
    }

    Ok(())
}
