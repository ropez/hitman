use std::fs;
use std::str;
use std::error::Error;
use toml::Table;

mod self::substitute;
use substitute::substitute;

