use core::any::{TypeId, Any};
use core::fmt::{Display, Debug};
use core::iter::IntoIterator;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut, Index, IndexMut};

use crate::clock::VideoTs;
use crate::memory::ZxMemory;
use super::{BusDevice, NullDevice};

/// A trait for dynamic bus devices, which currently includes methods from [Display] and [BusDevice].
/// Devices implementing this trait can be used with a [DynamicBusDevice].
///
/// Implemented for all types that implement dependent traits.
pub trait NamedBusDevice<T: Debug>: Display + BusDevice<Timestamp=T, NextDevice=NullDevice<T>>{}

impl<T: Debug, D> NamedBusDevice<T> for D where D: Display + BusDevice<Timestamp=T, NextDevice=NullDevice<T>> {}

/// A type of a dynamic [NamedBusDevice] with a constraint on a timestamp type.
pub type LinkedDynDevice<D> = dyn NamedBusDevice<<D as BusDevice>::Timestamp>;
/// This is a type of items stored by [DynamicBusDevice].
///
/// A type of a boxed dynamic [NamedBusDevice] with a constraint on a timestamp type.
pub type BoxLinkedDynDevice<D> = Box<dyn NamedBusDevice<<D as BusDevice>::Timestamp>>;

/// A bus device that allows for adding and removing devices of different types at run time.
///
/// The penalty is that the access to the devices must be done using a virtual call dispatch.
/// Also the device of this type can't be cloned (nor the [ControlUnit][crate::chip::ControlUnit]
/// with this device attached).
///
/// `DynamicBusDevice` implements [BusDevice] so obviously it's possible to attach a statically
/// dispatched next [BusDevice] to it. By default it is [NullDevice].
///
/// Currently only types implementing [BusDevice] terminated with [NullDevice] can be appended as
/// dynamically dispatched objects.
#[derive(Default, Debug)]
pub struct DynamicBusDevice<D: BusDevice=NullDevice<VideoTs>> {
    devices: Vec<BoxLinkedDynDevice<D>>,
    bus: D
}

impl<'a, T: Debug, D: 'a> From<D> for Box<dyn NamedBusDevice<T> + 'a>
    where D: BusDevice<Timestamp=T, NextDevice=NullDevice<T>> + Display
{
    fn from(dev: D) -> Self {
        Box::new(dev)
    }
}

impl<D: BusDevice> Deref for DynamicBusDevice<D> {
    type Target = [BoxLinkedDynDevice<D>];
    fn deref(&self) -> &Self::Target {
        &self.devices.as_slice()
    }
}

impl<D: BusDevice> DerefMut for DynamicBusDevice<D> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.devices.as_mut_slice()
    }
}

impl<D> DynamicBusDevice<D>
    where D: BusDevice
{
    /// Returns a number of attached devices.
    pub fn len(&self) -> usize {
        self.devices.len()
    }
    /// Appends a device at the end of the daisy-chain. Returns an index to a dynamic device.
    pub fn append_device<B>(&mut self, device: B) -> usize
        where B: Into<BoxLinkedDynDevice<D>>
    {
        self.devices.push(device.into());
        self.devices.len() - 1
    }
    /// Removes the last device from the dynamic daisy-chain and returns an instance of the boxed
    /// dynamic object.
    pub fn remove_device(&mut self) -> Option<BoxLinkedDynDevice<D>> {
        self.devices.pop()
    }
    /// Removes all dynamic devices from the dynamic daisy-chain.
    pub fn clear(&mut self) {
        self.devices.clear();
    }
    /// Returns a reference to a dynamic device at `index` in the dynamic daisy-chain.
    #[inline]
    pub fn get_device_ref(&self, index: usize) -> Option<&LinkedDynDevice<D>> {
        // self.devices[index].as_ref()
        self.devices.get(index).map(|d| d.as_ref())
    }
    /// Returns a mutable reference to a dynamic device at `index` in the dynamic daisy-chain.
    #[inline]
    pub fn get_device_mut(&mut self, index: usize) -> Option<&mut LinkedDynDevice<D>> {
        self.devices.get_mut(index).map(|d| d.as_mut())
    }
}

impl<D> DynamicBusDevice<D>
    where D: BusDevice, D::Timestamp: Debug + 'static
{
    /// Removes the last device from the dynamic daisy-chain.
    /// # Panics
    /// Panics if a device is not of a type given as parameter `B`.
    pub fn remove_as_device<B>(&mut self) -> Option<Box<B>>
        where B: NamedBusDevice<D::Timestamp> + 'static
    {
        self.remove_device().map(|boxdev|
            boxdev.downcast::<B>().expect("wrong dynamic device type removed")
        )
    }
    /// Returns a reference to a device of a type `B` at `index` in the dynamic daisy-chain.
    /// # Panics
    /// Panics if a device doesn't exist at `index` or if a device is not of a type given as parameter `B`.
    #[inline]
    pub fn as_device_ref<B>(&self, index: usize) -> &B
        where B: NamedBusDevice<D::Timestamp> + 'static
    {
        self.devices[index].downcast_ref::<B>().expect("wrong dynamic device type")
    }
    /// Returns a mutable reference to a device of a type `B` at `index` in the dynamic daisy-chain.
    /// # Panics
    /// Panics if a device doesn't exist at `index` or if a device is not of a type given as parameter `B`.
    #[inline]
    pub fn as_device_mut<B>(&mut self, index: usize) -> &mut B
        where B: NamedBusDevice<D::Timestamp> + 'static
    {
        self.devices[index].downcast_mut::<B>().expect("wrong dynamic device type")
    }
    /// Returns `true` if a device at `index` is of a type given as parameter `B`.
    #[inline]
    pub fn is_device<B>(&self, index: usize) -> bool
        where B: NamedBusDevice<D::Timestamp> + 'static
    {
        self.devices.get(index).map(|d| d.is::<B>()).unwrap_or(false)
    }
    /// Searches for a first device of a type given as parameter `B`, returning its index.
    #[inline]
    pub fn position_device<B>(&self) -> Option<usize>
        where B: NamedBusDevice<D::Timestamp> + 'static
    {
        self.devices.iter().position(|d| d.is::<B>())
    }
    /// Searches for a first device of a type given as parameter `B`, returning a reference to a device.
    #[inline]
    pub fn find_device_ref<B>(&self) -> Option<&B>
        where B: NamedBusDevice<D::Timestamp> + 'static
    {
        self.devices.iter().find_map(|d| d.downcast_ref::<B>())
    }
    /// Searches for a first device of a type given as parameter `B`, returning a mutable reference to a device.
    #[inline]
    pub fn find_device_mut<B>(&mut self) -> Option<&mut B>
        where B: NamedBusDevice<D::Timestamp> + 'static
    {
        self.devices.iter_mut().find_map(|d| d.downcast_mut::<B>())
    }
}

impl<D: BusDevice> Index<usize> for DynamicBusDevice<D> {
    type Output = LinkedDynDevice<D>;
    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        self.devices[index].as_ref()
    }
}

impl<D: BusDevice> IndexMut<usize> for DynamicBusDevice<D> {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.devices[index].as_mut()
    }
}

impl<'a, D: BusDevice> IntoIterator for &'a DynamicBusDevice<D> {
    type Item = &'a BoxLinkedDynDevice<D>;
    type IntoIter = core::slice::Iter<'a, BoxLinkedDynDevice<D>>;
    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.devices.iter()
    }
}

impl<'a, D: BusDevice> IntoIterator for &'a mut DynamicBusDevice<D> {
    type Item = &'a mut BoxLinkedDynDevice<D>;
    type IntoIter = core::slice::IterMut<'a, BoxLinkedDynDevice<D>>;
    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.devices.iter_mut()
    }
}

impl<D: BusDevice> IntoIterator for DynamicBusDevice<D> {
    type Item = BoxLinkedDynDevice<D>;
    type IntoIter = std::vec::IntoIter<BoxLinkedDynDevice<D>>;
    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.devices.into_iter()
    }
}

impl<D> BusDevice for DynamicBusDevice<D>
    where D: BusDevice, D::Timestamp: Debug + Copy
{
    type Timestamp = D::Timestamp;
    type NextDevice = D;

    #[inline]
    fn next_device_mut(&mut self) -> &mut Self::NextDevice {
        &mut self.bus
    }
    #[inline]
    fn next_device_ref(&self) -> &Self::NextDevice {
        &self.bus
    }
    #[inline]
    fn reset(&mut self, timestamp: Self::Timestamp) {
        for dev in self.devices.iter_mut() {
            dev.reset(timestamp);
        }
        self.bus.reset(timestamp);
    }
    #[inline]
    fn update_timestamp(&mut self, timestamp: Self::Timestamp) {
        for dev in self.devices.iter_mut() {
            dev.update_timestamp(timestamp);
        }
        self.bus.update_timestamp(timestamp);
    }
    #[inline]
    fn next_frame(&mut self, timestamp: Self::Timestamp) {
        for dev in self.devices.iter_mut() {
            dev.next_frame(timestamp);
        }
        self.bus.next_frame(timestamp);
    }
    #[inline]
    fn read_io(&mut self, port: u16, timestamp: Self::Timestamp) -> Option<u8> {
        let mut bus_data = self.bus.read_io(port, timestamp);
        for dev in self.devices.iter_mut() {
            if let Some(data) = dev.read_io(port, timestamp) {
                bus_data = Some(data & bus_data.unwrap_or(!0));
            }
        }
        bus_data
    }
    #[inline]
    fn write_io(&mut self, port: u16, data: u8, timestamp: Self::Timestamp) -> bool {
        for dev in self.devices.iter_mut() {
            if dev.write_io(port, data, timestamp) {
                return true;
            }
        }
        self.bus.write_io(port, data, timestamp)
    }
}

#[cfg(test)]
mod tests {
    use core::fmt;
    use super::*;

    #[derive(Default, Clone, PartialEq, Debug)]
    struct TestDevice {
        foo: i32,
        data: u8,
        bus: NullDevice<i32>
    }

    impl fmt::Display for TestDevice {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("Test Device")
        }
    }

    impl BusDevice for TestDevice {
        type Timestamp = i32;
        type NextDevice = NullDevice<i32>;

        fn next_device_mut(&mut self) -> &mut Self::NextDevice {
            &mut self.bus
        }
        fn next_device_ref(&self) -> &Self::NextDevice {
            &self.bus
        }
        fn reset(&mut self, timestamp: Self::Timestamp) {
            self.foo = i32::min_value() + timestamp;
        }
        fn update_timestamp(&mut self, timestamp: Self::Timestamp) {
            self.foo = timestamp
        }
        fn read_io(&mut self, _port: u16, timestamp: Self::Timestamp) -> Option<u8> {
            if self.foo == timestamp {
                Some(self.data)
            }
            else {
                None
            }
        }
        fn write_io(&mut self, _port: u16, data: u8, timestamp: Self::Timestamp) -> bool {
            self.data = data;
            self.foo = timestamp;
            true
        }
    }

    #[test]
    fn dynamic_bus_device_works() {
        let mut dchain: DynamicBusDevice<NullDevice<i32>> = Default::default();
        assert_eq!(dchain.len(), 0);
        assert_eq!(dchain.write_io(0, 0, 0), false);
        assert_eq!(dchain.read_io(0, 0), None);
        let test_dev: Box<dyn NamedBusDevice<_>> = Box::new(TestDevice::default());
        let index = dchain.append_device(test_dev);
        assert_eq!(dchain.is_device::<TestDevice>(index), true);
        assert_eq!(index, 0);
        assert_eq!(dchain.len(), 1);
        let device = dchain.remove_device().unwrap();
        assert_eq!(device.is::<TestDevice>(), true);
        assert_eq!(dchain.len(), 0);

        let index0 = dchain.append_device(NullDevice::default());
        assert_eq!(index0, 0);
        assert_eq!(dchain.is_device::<TestDevice>(index0), false);
        assert_eq!(dchain.is_device::<NullDevice<_>>(index0), true);
        assert_eq!(dchain.len(), 1);
        let dev: &NullDevice<_> = dchain.as_device_ref(index0);
        assert_eq!(dev, &NullDevice::default());

        let index1 = dchain.append_device(TestDevice::default());
        assert_eq!(index1, 1);
        assert_eq!(dchain.is_device::<TestDevice>(index1), true);
        assert_eq!(dchain.is_device::<TestDevice>(index0), false);
        assert_eq!(dchain.is_device::<TestDevice>(usize::max_value()), false);
        assert_eq!(dchain.is_device::<NullDevice<_>>(index0), true);
        assert_eq!(dchain.is_device::<NullDevice<_>>(index1), false);
        assert_eq!(dchain.is_device::<NullDevice<_>>(usize::max_value()), false);
        let dev = dchain.get_device_ref(index1).unwrap();
        assert_eq!(dev.is::<TestDevice>(), true);
        assert_eq!(dev.is::<NullDevice<_>>(), false);
        assert_eq!(format!("{}", dev), "Test Device");
        if let Some(dev) = dchain.get_device_mut(index1) {
            dev.update_timestamp(777);
            assert_eq!(dev.read_io(0, 0), None);
            assert_eq!(dev.read_io(0, 777), Some(0));
        }
        assert_eq!(dchain.len(), 2);
        assert_eq!(dchain.write_io(0, 42, 131999), true);
        assert_eq!(dchain.read_io(0, 0), None);
        assert_eq!(dchain.read_io(0, 131999), Some(42));
        let dev: &TestDevice = dchain.as_device_ref(index1);
        assert_eq!(dev.data, 42);
        assert_eq!(dev.foo, 131999);
        let dev: &mut TestDevice = dchain.as_device_mut(index1);
        dev.data = 199;
        dev.foo = -1;
        let dev: &TestDevice = dchain.as_device_ref(index1);
        assert_eq!(dev.data, 199);
        assert_eq!(dev.foo, -1);
        let device: TestDevice = *dchain.remove_as_device().unwrap();
        assert_eq!(dchain.len(), 1);
        assert_eq!(device, TestDevice {
            foo: -1,
            data: 199,
            bus: NullDevice::<i32>::default()
        });
    }
}
