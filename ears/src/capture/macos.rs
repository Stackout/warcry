use anyhow::{Result, anyhow, bail};
use crossbeam_channel::Sender;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_core_audio::{
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID, AudioDeviceStart,
    AudioDeviceStop, AudioHardwareCreateAggregateDevice, AudioHardwareCreateProcessTap,
    AudioHardwareDestroyAggregateDevice, AudioHardwareDestroyProcessTap,
    AudioObjectGetPropertyData, AudioObjectID, AudioObjectPropertyAddress, CATapDescription,
    kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceNameKey,
    kAudioAggregateDeviceTapAutoStartKey, kAudioAggregateDeviceTapListKey,
    kAudioAggregateDeviceUIDKey, kAudioHardwarePropertyTranslatePIDToProcessObject,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
    kAudioSubTapUIDKey, kAudioTapPropertyFormat,
};
use objc2_core_audio_types::{AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp};
use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType, Type};
use objc2_foundation::{NSArray, NSNumber, NSString};
use std::ffi::{CStr, c_void};
use std::ptr::NonNull;

pub struct Tap {
    tap: AudioObjectID,
    device: AudioObjectID,
    io_proc: AudioDeviceIOProcID,
    sink: *mut Sink,
    pub rate: u32,
    pub channels: usize,
}

unsafe impl Send for Tap {}

struct Sink {
    tx: Sender<Vec<f32>>,
}

pub fn open(pid: u32, tx: Sender<Vec<f32>>) -> Result<Tap> {
    let process = process_object(pid)?;
    let description = describe(process, pid);
    let uid = tap_uid(&description);
    let tap = create_tap(&description)?;
    attach(tap, &uid, pid, tx).inspect_err(|_| destroy_tap(tap))
}

fn attach(tap: AudioObjectID, uid: &str, pid: u32, tx: Sender<Vec<f32>>) -> Result<Tap> {
    let format = tap_format(tap)?;
    let device = create_aggregate(uid, pid)?;
    let sink = Box::into_raw(Box::new(Sink { tx }));
    let mut io_proc: AudioDeviceIOProcID = None;
    match start(device, sink, &mut io_proc) {
        Err(e) => {
            unwind(device, io_proc, sink);
            Err(e)
        }
        Ok(()) => Ok(Tap {
            tap,
            device,
            io_proc,
            sink,
            rate: format.mSampleRate as u32,
            channels: format.mChannelsPerFrame.max(1) as usize,
        }),
    }
}

fn start(device: AudioObjectID, sink: *mut Sink, io_proc: &mut AudioDeviceIOProcID) -> Result<()> {
    let created = unsafe {
        AudioDeviceCreateIOProcID(
            device,
            Some(io_proc_callback),
            sink.cast(),
            NonNull::from(&mut *io_proc),
        )
    };
    check(created, "AudioDeviceCreateIOProcID")?;
    check(
        unsafe { AudioDeviceStart(device, *io_proc) },
        "AudioDeviceStart",
    )
}

fn unwind(device: AudioObjectID, io_proc: AudioDeviceIOProcID, sink: *mut Sink) {
    unsafe {
        AudioDeviceDestroyIOProcID(device, io_proc);
        AudioHardwareDestroyAggregateDevice(device);
        drop(Box::from_raw(sink));
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        unsafe { AudioDeviceStop(self.device, self.io_proc) };
        unwind(self.device, self.io_proc, self.sink);
        destroy_tap(self.tap);
    }
}

unsafe extern "C-unwind" fn io_proc_callback(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    _output: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client: *mut c_void,
) -> i32 {
    let sink = unsafe { &*client.cast::<Sink>() };
    let list = unsafe { input.as_ref() };
    let buffers =
        unsafe { std::slice::from_raw_parts(list.mBuffers.as_ptr(), list.mNumberBuffers as usize) };
    let mono = buffers.first().map(|buffer| {
        let count = buffer.mDataByteSize as usize / size_of::<f32>();
        let samples = unsafe { std::slice::from_raw_parts(buffer.mData.cast::<f32>(), count) };
        crate::resample::downmix(samples, buffer.mNumberChannels.max(1) as usize)
    });
    mono.map(|frame| sink.tx.try_send(frame));
    0
}

fn process_object(pid: u32) -> Result<AudioObjectID> {
    let address = address(kAudioHardwarePropertyTranslatePIDToProcessObject);
    let target = pid as i32;
    let mut object: AudioObjectID = 0;
    let mut size = size_of::<AudioObjectID>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as u32,
            NonNull::from(&address),
            size_of::<i32>() as u32,
            (&raw const target).cast(),
            NonNull::from(&mut size),
            NonNull::from(&mut object).cast(),
        )
    };
    check(status, "translate pid to audio process")?;
    match object {
        0 => bail!("process {pid} has no audio to tap; is the game making sound?"),
        _ => Ok(object),
    }
}

fn address(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

fn describe(process: AudioObjectID, pid: u32) -> Retained<CATapDescription> {
    let processes = NSArray::from_retained_slice(&[NSNumber::new_u32(process)]);
    let description = unsafe {
        CATapDescription::initMonoMixdownOfProcesses(CATapDescription::alloc(), &processes)
    };
    unsafe { description.setName(&NSString::from_str(&format!("Warcry {pid}"))) };
    unsafe { description.setPrivate(true) };
    description
}

fn tap_uid(description: &CATapDescription) -> String {
    unsafe { description.UUID().UUIDString() }.to_string()
}

fn create_tap(description: &CATapDescription) -> Result<AudioObjectID> {
    let mut tap: AudioObjectID = 0;
    let status = unsafe { AudioHardwareCreateProcessTap(Some(description), &mut tap) };
    check(status, "AudioHardwareCreateProcessTap")?;
    Ok(tap)
}

fn destroy_tap(tap: AudioObjectID) {
    unsafe { AudioHardwareDestroyProcessTap(tap) };
}

fn tap_format(tap: AudioObjectID) -> Result<AudioStreamBasicDescription> {
    let address = address(kAudioTapPropertyFormat);
    let mut format: AudioStreamBasicDescription = unsafe { std::mem::zeroed() };
    let mut size = size_of::<AudioStreamBasicDescription>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            tap,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut format).cast(),
        )
    };
    check(status, "kAudioTapPropertyFormat")?;
    Ok(format)
}

fn create_aggregate(uid: &str, pid: u32) -> Result<AudioObjectID> {
    let sub_key = key(kAudioSubTapUIDKey);
    let sub_value = erase(CFString::from_str(uid));
    let sub_tap: CFRetained<CFDictionary<CFString, CFType>> =
        CFDictionary::from_slices(&[&*sub_key], &[&*sub_value]);
    let taps = CFArray::from_retained_objects(&[erase(sub_tap)]);
    let keys = [
        key(kAudioAggregateDeviceNameKey),
        key(kAudioAggregateDeviceUIDKey),
        key(kAudioAggregateDeviceIsPrivateKey),
        key(kAudioAggregateDeviceTapAutoStartKey),
        key(kAudioAggregateDeviceTapListKey),
    ];
    let values = [
        erase(CFString::from_str(&format!("Warcry {pid}"))),
        erase(CFString::from_str(&format!(
            "com.stackout.warcry.tap.{pid}.{uid}"
        ))),
        erase(CFNumber::new_i32(1)),
        erase(CFNumber::new_i32(1)),
        erase(taps),
    ];
    let description: CFRetained<CFDictionary<CFString, CFType>> = CFDictionary::from_slices(
        &keys.each_ref().map(|k| &**k),
        &values.each_ref().map(|v| &**v),
    );
    let mut device: AudioObjectID = 0;
    let status = unsafe {
        AudioHardwareCreateAggregateDevice(
            &CFRetained::cast_unchecked(description),
            NonNull::from(&mut device),
        )
    };
    check(status, "AudioHardwareCreateAggregateDevice")?;
    Ok(device)
}

fn erase<T: Type>(value: CFRetained<T>) -> CFRetained<CFType> {
    unsafe { CFRetained::cast_unchecked(value) }
}

fn key(name: &CStr) -> CFRetained<CFString> {
    CFString::from_str(name.to_str().unwrap_or_default())
}

fn check(status: i32, what: &str) -> Result<()> {
    match status {
        0 => Ok(()),
        _ => Err(anyhow!("{what} failed: {}", fourcc(status))),
    }
}

fn fourcc(status: i32) -> String {
    let tag: String = status
        .to_be_bytes()
        .iter()
        .map(|b| match b.is_ascii_graphic() {
            true => *b as char,
            false => '.',
        })
        .collect();
    format!("OSStatus {status} ({tag})")
}
