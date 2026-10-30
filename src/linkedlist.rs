use core::{
    cell::{Cell, RefCell, UnsafeCell},
    future::poll_fn,
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    ops::{Deref, DerefMut},
    task::Poll,
};

use embassy_sync::{
    blocking_mutex::{Mutex, raw::RawMutex},
    semaphore::{FairSemaphore, Semaphore, WaitQueueFull},
    waitqueue::WakerRegistration,
};
use nonmax::NonMaxU8;

pub(crate) type NodeIndex = NonMaxU8;

pub trait Reset {
    fn clear(&mut self);
}

pub(crate) struct Node {
    next: Option<NodeIndex>,
}

impl Node {
    const fn new<const N: usize>(index: usize) -> Self {
        const {
            assert!(N <= NodeIndex::MAX.get() as usize + 1);
        }
        assert!(index < N);

        Self {
            next: if index + 1 < N {
                NodeIndex::new((index + 1) as u8)
            } else {
                None
            },
        }
    }
}

pub(crate) struct IndexList {
    head: Option<NodeIndex>,
    tail: Option<NodeIndex>,
    len: usize,
}

impl IndexList {
    const fn new() -> Self {
        Self {
            head: None,
            tail: None,
            len: 0,
        }
    }

    unsafe fn pop(&mut self, nodes: &[UnsafeCell<Node>]) -> Option<NodeIndex> {
        let index = self.head?;
        let node = unsafe { &mut *nodes[index.get() as usize].get() };

        self.head = node.next.take();
        self.len -= 1;

        if self.head.is_none() {
            self.tail = None;
        }

        Some(index)
    }

    unsafe fn push(&mut self, nodes: &[UnsafeCell<Node>], index: NodeIndex) {
        unsafe {
            (*nodes[index.get() as usize].get()).next = None;
        }

        if let Some(tail) = self.tail {
            unsafe {
                (*nodes[tail.get() as usize].get()).next = Some(index);
            }
        } else {
            self.head = Some(index);
        }

        self.tail = Some(index);
        self.len += 1;
    }
}

pub(crate) trait PoolAccess: 'static {
    type Item: Reset + 'static;

    fn try_acquire_index(&self) -> Option<NodeIndex>;

    // SAFETY: 호출자는 해당 슬롯의 소유권과 참조의 독점성을 보장해야 한다.
    unsafe fn data_ptr(&self, index: NodeIndex) -> *mut Self::Item;

    // SAFETY: 호출자는 슬롯을 독점 소유하고 한 번만 반환해야 한다.
    // 반환 후에는 슬롯에 접근하면 안 된다.
    unsafe fn release_index(&self, index: NodeIndex);

    fn nodes(&self) -> &[UnsafeCell<Node>];

    unsafe fn release_list(&self, list: IndexList);
}

pub(crate) trait WaitPoolAccess: PoolAccess {
    async fn acquire_index(&self) -> Result<NodeIndex, WaitQueueFull>;
}

pub(crate) trait StaticPool: Sized + 'static {
    type Storage: PoolAccess;

    // 같은 풀 타입은 항상 같은 저장소를 반환해야 한다.
    fn storage() -> &'static Self::Storage;
}

pub(crate) struct ItemHandle<P: StaticPool> {
    index: NodeIndex,
    _pool: PhantomData<P>,
    _not_sync: PhantomData<Cell<()>>,
}

impl<P: StaticPool> ItemHandle<P> {
    fn from_index(index: NodeIndex) -> Self {
        Self {
            index,
            _pool: PhantomData,
            _not_sync: PhantomData,
        }
    }
}

impl<P: StaticPool> Deref for ItemHandle<P> {
    type Target = <P::Storage as PoolAccess>::Item;

    fn deref(&self) -> &Self::Target {
        // SAFETY: 핸들이 슬롯을 소유하며 참조 수명은 핸들 대여에 묶인다.
        unsafe { &*P::storage().data_ptr(self.index) }
    }
}

impl<P: StaticPool> DerefMut for ItemHandle<P> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: 핸들과 슬롯을 독점 접근하며 참조 수명은 대여에 묶인다.
        unsafe { &mut *P::storage().data_ptr(self.index) }
    }
}

impl<P: StaticPool> Drop for ItemHandle<P> {
    fn drop(&mut self) {
        // SAFETY: 핸들이 소유한 슬롯을 한 번 반환한다.
        unsafe { P::storage().release_index(self.index) }
    }
}

impl<P: StaticPool> AsRef<[u8]> for ItemHandle<P>
where
    <P::Storage as PoolAccess>::Item: AsRef<[u8]>,
{
    fn as_ref(&self) -> &[u8] {
        (**self).as_ref()
    }
}

pub(crate) struct PoolInner<T, R: RawMutex, const N: usize> {
    data: [UnsafeCell<T>; N],
    nodes: [UnsafeCell<Node>; N],
    free_list: Mutex<R, RefCell<IndexList>>,
}

impl<T: Reset, R: RawMutex, const N: usize> PoolInner<T, R, N> {
    pub(crate) const fn new(data: [UnsafeCell<T>; N]) -> Self {
        const {
            assert!(N <= NodeIndex::MAX.get() as usize + 1, "invalid pool size");
        }

        let head = if N > 0 { NodeIndex::new(0) } else { None };

        let tail = if N > 0 {
            NodeIndex::new((N - 1) as u8)
        } else {
            None
        };

        let mut nodes = [const { UnsafeCell::new(Node { next: None }) }; N];
        let mut index = 0;
        while index < N {
            nodes[index] = UnsafeCell::new(Node::new::<N>(index));
            index += 1;
        }

        Self {
            data,
            nodes,
            free_list: Mutex::new(RefCell::new(IndexList { head, tail, len: N })),
        }
    }

    fn try_take_index(&self) -> Option<NodeIndex> {
        let index = self
            .free_list
            .lock(|state| unsafe { state.borrow_mut().pop(&self.nodes) })?;

        unsafe {
            (&mut *self.data[index.get() as usize].get()).clear();
        }

        Some(index)
    }

    unsafe fn data_ptr(&self, index: NodeIndex) -> *mut T {
        self.data[index.get() as usize].get()
    }

    unsafe fn release_index(&self, index: NodeIndex) {
        unsafe {
            (&mut *self.data[index.get() as usize].get()).clear();
        }

        self.free_list.lock(|state| unsafe {
            state.borrow_mut().push(&self.nodes, index);
        });
    }

    fn nodes(&self) -> &[UnsafeCell<Node>] {
        &self.nodes
    }

    #[allow(dead_code)]
    unsafe fn release_list(&self, list: IndexList) -> usize {
        let Some(head) = list.head else {
            return 0;
        };
        let tail = list.tail.expect("nonempty list without tail");

        self.free_list.lock(|state| {
            let mut free = state.borrow_mut();

            if let Some(free_tail) = free.tail {
                // SAFETY: 기존 free list는 잠금으로 보호되고,
                // 반환 체인은 독점 소유한다.
                unsafe {
                    (*self.nodes[free_tail.get() as usize].get()).next = Some(head);
                }
            } else {
                free.head = Some(head);
            }

            free.tail = Some(tail);
            free.len += list.len;
        });

        list.len
    }
}

unsafe impl<T: Send, R: RawMutex + Sync, const N: usize> Sync for PoolInner<T, R, N> {}

pub(crate) struct TxPool<T, R: RawMutex, const N: usize, const W: usize> {
    inner: PoolInner<T, R, N>,
    waiters: FairSemaphore<R, W>,
}

impl<T, R, const N: usize, const W: usize> TxPool<T, R, N, W>
where
    T: Reset + 'static,
    R: RawMutex + 'static,
{
    pub(crate) const fn new(data: [UnsafeCell<T>; N]) -> Self {
        Self {
            inner: PoolInner::new(data),
            waiters: FairSemaphore::new(N),
        }
    }
}

impl<T, R, const N: usize, const W: usize> PoolAccess for TxPool<T, R, N, W>
where
    T: Reset + 'static,
    R: RawMutex + 'static,
{
    type Item = T;

    fn try_acquire_index(&self) -> Option<NodeIndex> {
        let permit = self.waiters.try_acquire(1)?;
        let index = self
            .inner
            .try_take_index()
            .expect("semaphore acquired but empty free list");

        permit.disarm();
        Some(index)
    }

    unsafe fn data_ptr(&self, index: NodeIndex) -> *mut Self::Item {
        unsafe { self.inner.data_ptr(index) }
    }

    unsafe fn release_index(&self, index: NodeIndex) {
        unsafe {
            self.inner.release_index(index);
        }

        self.waiters.release(1);
    }

    fn nodes(&self) -> &[UnsafeCell<Node>] {
        self.inner.nodes()
    }

    unsafe fn release_list(&self, list: IndexList) {
        let released = unsafe { self.inner.release_list(list) };

        self.waiters.release(released);
    }
}

impl<T, R, const N: usize, const W: usize> WaitPoolAccess for TxPool<T, R, N, W>
where
    T: Reset + 'static,
    R: RawMutex + 'static,
{
    async fn acquire_index(&self) -> Result<NodeIndex, WaitQueueFull> {
        let permit = self.waiters.acquire(1).await?;
        let index = self
            .inner
            .try_take_index()
            .expect("semaphore acquired but empty free list");

        permit.disarm();
        Ok(index)
    }
}

// RxPool is only acquired by Link, so waiter is not needed
pub(crate) struct RxPool<T, R: RawMutex, const N: usize> {
    inner: PoolInner<T, R, N>,
}

impl<T, R, const N: usize> RxPool<T, R, N>
where
    T: Reset + 'static,
    R: RawMutex + 'static,
{
    pub(crate) const fn new(data: [UnsafeCell<T>; N]) -> Self {
        Self {
            inner: PoolInner::new(data),
        }
    }
}

impl<T, R, const N: usize> PoolAccess for RxPool<T, R, N>
where
    T: Reset + 'static,
    R: RawMutex + 'static,
{
    type Item = T;

    fn try_acquire_index(&self) -> Option<NodeIndex> {
        self.inner.try_take_index()
    }

    unsafe fn data_ptr(&self, index: NodeIndex) -> *mut Self::Item {
        unsafe { self.inner.data_ptr(index) }
    }

    unsafe fn release_index(&self, index: NodeIndex) {
        unsafe {
            self.inner.release_index(index);
        }
    }

    fn nodes(&self) -> &[UnsafeCell<Node>] {
        self.inner.nodes()
    }

    unsafe fn release_list(&self, list: IndexList) {
        let _released = unsafe { self.inner.release_list(list) };
    }
}

struct ListState {
    list: IndexList,
    waker: WakerRegistration,
}

pub(crate) struct PoolLinkedList<P: StaticPool, R: RawMutex> {
    state: Mutex<R, RefCell<ListState>>,
    _pool: PhantomData<P>,
}

impl<P: StaticPool, R: RawMutex> PoolLinkedList<P, R> {
    pub(crate) const fn new() -> Self {
        Self {
            state: Mutex::new(RefCell::new(ListState {
                list: IndexList::new(),
                waker: WakerRegistration::new(),
            })),
            _pool: PhantomData,
        }
    }

    pub(crate) fn try_receive(&self) -> Option<ItemHandle<P>> {
        let index = self.state.lock(|state| {
            let mut state = state.borrow_mut();

            // SAFETY: 리스트 소속 노드는 이 잠금으로 보호된다.
            unsafe { state.list.pop(P::storage().nodes()) }
        })?;

        Some(ItemHandle::from_index(index))
    }

    pub(crate) async fn receive(&self) -> ItemHandle<P> {
        let index = poll_fn(|cx| {
            self.state.lock(|state| {
                let mut state = state.borrow_mut();

                // SAFETY: 리스트 소속 노드는 이 잠금으로 보호된다.
                if let Some(index) = unsafe { state.list.pop(P::storage().nodes()) } {
                    Poll::Ready(index)
                } else {
                    state.waker.register(cx.waker());
                    Poll::Pending
                }
            })
        })
        .await;

        ItemHandle::from_index(index)
    }

    pub(crate) fn send(&self, item: ItemHandle<P>) {
        // 핸들을 소비하되 슬롯을 풀에 반환하지 않는다.
        let index = ManuallyDrop::new(item).index;

        self.state.lock(|state| {
            let mut state = state.borrow_mut();

            // SAFETY: 같은 풀의 핸들에서 소유권을 넘겨받았고,
            // 기존 리스트 소속 노드는 이 잠금으로 보호된다.
            unsafe {
                state.list.push(P::storage().nodes(), index);
            }

            state.waker.wake();
        });
    }

    pub(crate) fn try_acquire(&self) -> Option<ItemHandle<P>> {
        P::storage().try_acquire_index().map(ItemHandle::from_index)
    }

    pub(crate) fn sender(&self) -> Sender<'_, Self> {
        Sender { queue: self }
    }

    pub(crate) fn receiver(&self) -> Receiver<'_, Self> {
        Receiver { queue: self }
    }

    pub(crate) fn clear(&self) {
        let list = self
            .state
            .lock(|state| core::mem::replace(&mut state.borrow_mut().list, IndexList::new()));

        // SAFETY: 큐에서 분리한 체인을 독점 소유한다.
        unsafe { P::storage().release_list(list) };
    }

    pub(crate) fn size(&self) -> usize {
        self.state.lock(|state| state.borrow().list.len)
    }
}

impl<P: StaticPool, R: RawMutex> PoolLinkedList<P, R>
where
    P::Storage: WaitPoolAccess,
{
    pub(crate) async fn acquire(&self) -> Result<ItemHandle<P>, WaitQueueFull> {
        let index = P::storage().acquire_index().await?;
        Ok(ItemHandle::from_index(index))
    }
}

pub(crate) struct Sender<'q, Q> {
    pub(crate) queue: &'q Q,
}

impl<'q, P: StaticPool, R: RawMutex> Sender<'q, PoolLinkedList<P, R>> {
    pub(crate) fn send(&self, item: ItemHandle<P>) {
        self.queue.send(item);
    }

    pub(crate) fn try_acquire(&self) -> Option<ItemHandle<P>> {
        self.queue.try_acquire()
    }
}

impl<'q, P, R> Sender<'q, PoolLinkedList<P, R>>
where
    P: StaticPool,
    P::Storage: WaitPoolAccess,
    R: RawMutex,
{
    pub(crate) async fn acquire(&self) -> Result<ItemHandle<P>, WaitQueueFull> {
        self.queue.acquire().await
    }
}

pub(crate) struct Receiver<'q, Q> {
    pub(crate) queue: &'q Q,
}

impl<'q, P: StaticPool, R: RawMutex> Receiver<'q, PoolLinkedList<P, R>> {
    pub(crate) fn try_receive(&self) -> Option<ItemHandle<P>> {
        self.queue.try_receive()
    }

    pub(crate) async fn receive(&self) -> ItemHandle<P> {
        self.queue.receive().await
    }
}

macro_rules! static_pool {
    (
        $vis:vis $name:ident:
        $storage:ty,
        init: $init:expr $(,)?
    ) => {
        $vis struct $name;

        impl $crate::linkedlist::StaticPool for $name {
            type Storage = $storage;

            fn storage() -> &'static Self::Storage {
                static STORAGE: $storage = $init;

                &STORAGE
            }
        }

        impl $name {
            // SAFETY: list는 이 풀에서 분리한 독점 소유 체인이어야 한다.
            #[allow(dead_code)]
            pub(crate) unsafe fn release_list(
                list: $crate::linkedlist::IndexList,
            ) {
                let storage =
                    <Self as $crate::linkedlist::StaticPool>::storage();

                unsafe {
                    $crate::linkedlist::PoolAccess::release_list(
                        storage,
                        list,
                    );
                }
            }
        }
    };
}

pub(crate) use static_pool;
