use crate::utils::get_param;
use std::collections::HashMap;

pub(crate) struct Params {
    pub vote_duration: u64,
    pub ping_interval: u64,
    pub duration_before: u64,
    pub duration_after: u64,
}

impl Params {
    pub(crate) fn new(
        instance_params: &HashMap<String, String>,
    ) -> Result<Params, Box<dyn std::error::Error>> {
        Ok(Params {
            vote_duration: get_param::<u64>("vote_duration", instance_params)?,
            ping_interval: get_param::<u64>("ping_interval", instance_params)?,
            duration_before: get_param::<u64>("duration_before", instance_params)?,
            duration_after: get_param::<u64>("duration_after", instance_params)?,
        })
    }
}
