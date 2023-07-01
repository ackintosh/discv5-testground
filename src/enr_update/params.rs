use crate::utils::get_param;
use std::collections::HashMap;

pub(crate) struct Params {
    pub ping_interval: u64,
}

impl Params {
    pub(crate) fn new(
        instance_params: &HashMap<String, String>,
    ) -> Result<Params, Box<dyn std::error::Error>> {
        Ok(Params {
            ping_interval: get_param::<u64>("ping_interval", instance_params)?,
        })
    }
}
