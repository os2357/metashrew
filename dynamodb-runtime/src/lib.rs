use anyhow::Result;
use log::{debug, info};
use metashrew_runtime::{BatchLike, KeyValueStoreLike};
use redis::Commands;
use std::sync::{Arc, Mutex};

const TIP_HEIGHT_KEY: &'static str = "/__INTERNAL/tip-height";

pub struct RedisRuntimeAdapter(pub String, pub Arc<Mutex<redis::Connection>>, pub u32);

static mut _LABEL: Option<String> = None;

const TIMEOUT: u64 = 1500;

use std::{thread, time};

pub fn wait_timeout() {
    thread::sleep(time::Duration::from_millis(TIMEOUT));
}

pub fn set_label(s: String) -> () {
    unsafe {
        _LABEL = Some(s + "://");
    }
}

pub fn get_label() -> &'static String {
    unsafe { _LABEL.as_ref().unwrap() }
}

pub fn has_label() -> bool {
    unsafe { _LABEL.is_some() }
}

pub fn to_labeled_key(key: &Vec<u8>) -> Vec<u8> {
    if has_label() {
        let mut result: Vec<u8> = vec![];
        result.extend(get_label().as_str().as_bytes());
        result.extend(key);
        result
    } else {
        key.clone()
    }
}

pub async fn query_height(connection: &mut redis::Connection, start_block: u32) -> Result<u32> {
    let height_key = TIP_HEIGHT_KEY.as_bytes().to_vec();
    let bytes: Vec<u8> = match connection.get(&to_labeled_key(&height_key)) {
        Ok(v) => v,
        Err(_) => {
            return Ok(start_block);
        }
    };
    if bytes.len() == 0 {
        return Ok(start_block);
    }
    let bytes_ref: &[u8] = &bytes;
    Ok(u32::from_le_bytes(bytes_ref.try_into().unwrap()))
}

impl RedisRuntimeAdapter {
    pub fn connect_once(&self) -> Result<redis::Connection> {
        Ok(redis::Client::open(self.0.clone())?.get_connection()?)
    }
    pub fn open(redis_uri: String) -> Result<RedisRuntimeAdapter> {
        Ok(RedisRuntimeAdapter(
            redis_uri.clone(),
            Arc::new(Mutex::new(
                redis::Client::open(redis_uri.clone())?.get_connection()?,
            )),
            0,
        ))
    }
    pub fn connect(&self) -> Result<redis::Connection> {
        loop {
            match self.connect_once() {
                Ok(v) => {
                    return Ok(v);
                }
                Err(e) => {
                    debug!("{:?}", e);
                    debug!("KeyDB reset -- wait 1.5s");
                    wait_timeout();
                }
            }
        }
    }
    pub fn reset_connection(&mut self) {
        debug!("KeyDB reset -- wait 1.5s");
        wait_timeout();
        self.1 = Arc::new(Mutex::new(self.connect().unwrap()));
    }
}

pub struct RedisBatch(pub redis::Pipeline);

fn to_redis_key<T: AsRef<[u8]>>(v: T) -> Vec<Vec<u8>> {
    if has_label() {
        let mut data: Vec<u8> = get_label().as_str().as_bytes().to_vec();
        let key: &[u8] = v.as_ref().try_into().unwrap();
        data.extend(key);
        vec![data]
    } else {
        to_redis_args(v)
    }
}
fn to_redis_args<T: AsRef<[u8]>>(v: T) -> Vec<Vec<u8>> {
    return vec![v.as_ref().try_into().unwrap()];
}

impl BatchLike for RedisBatch {
    fn default() -> Self {
        Self(redis::pipe())
    }
    fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(&mut self, k: K, v: V) {
        self.0
            .cmd("SET")
            .arg(to_redis_key(k))
            .arg(to_redis_args(v))
            .ignore();
    }
}

impl Clone for RedisRuntimeAdapter {
    fn clone(&self) -> Self {
        return Self(self.0.clone(), self.1.clone(), self.2);
    }
}

impl KeyValueStoreLike for RedisRuntimeAdapter {
    type Batch = RedisBatch;
    type Error = redis::RedisError;
    fn write(&mut self, mut batch: RedisBatch) -> Result<(), Self::Error> {
        let key_bytes: Vec<u8> = TIP_HEIGHT_KEY.as_bytes().to_vec();
        let height_bytes: Vec<u8> = (self.2 + 1).to_le_bytes().to_vec();
        /*
        let _ok: bool = connection
            .set(to_redis_args(&key_bytes), to_redis_args(&height_bytes))
            .unwrap();
            */
        batch.put(&key_bytes, &height_bytes);
        loop {
            {
                match batch.0.query::<()>(&mut self.1.lock().unwrap()) {
                    Ok(_) => {
                        return Ok(());
                    }
                    Err(e) => {
                        debug!("{:?}", e);
                    }
                }
            }
            self.reset_connection();
        }
    }
    fn get<K: AsRef<[u8]>>(&mut self, key: K) -> Result<Option<Vec<u8>>, Self::Error> {
        loop {
            {
                match self
                    .1
                    .lock()
                    .unwrap()
                    .get::<Vec<Vec<u8>>, Option<Vec<u8>>>(to_redis_args(key.as_ref()))
                {
                    Ok(v) => return Ok(v),
                    Err(e) => {
                        debug!("{:?}", e);
                    }
                }
            }
            self.reset_connection();
        }
    }
    fn delete<K: AsRef<[u8]>>(&mut self, key: K) -> Result<(), Self::Error> {
        loop {
            {
                match self
                    .1
                    .lock()
                    .unwrap()
                    .del::<Vec<Vec<u8>>, ()>(to_redis_args(key.as_ref()))
                {
                    Ok(_) => {
                        return Ok(());
                    }
                    Err(e) => {
                        debug!("{:?}", e);
                    }
                }
            }
            self.reset_connection();
        }
    }
    fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(&mut self, key: K, value: V) -> Result<(), Self::Error> {
        loop {
            {
                match self
                    .1
                    .lock()
                    .unwrap()
                    .set::<Vec<Vec<u8>>, Vec<Vec<u8>>, ()>(
                        to_redis_key(key.as_ref()),
                        to_redis_args(value.as_ref()),
                    ) {
                    Ok(v) => {
                        return Ok(());
                    }
                    Err(e) => {
                        debug!("{:?}", e);
                    }
                }
            }
            self.reset_connection();
        }
    }
}
