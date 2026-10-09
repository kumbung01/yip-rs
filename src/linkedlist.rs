use core::{
    cell::{Cell, RefCell, UnsafeCell},
    future::poll_fn,
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
    task::Poll,
};
use embassy_sync::{
    blocking_mutex::{Mutex, raw::RawMutex},
    semaphore::{FairSemaphore, Semaphore, WaitQueueFull},
    waitqueue::WakerRegistration,
};
use nonmax::NonMaxU8;

type NodeIndex = NonMaxU8;

struct Node {
    next: Option<NodeIndex>,
}

impl Node {
    fn new<const POOL_SIZE: usize>(index: usize) -> Self {
        const {
            assert!(POOL_SIZE <= NodeIndex::MAX.get() as usize + 1);
        }
        assert!(index < POOL_SIZE);

        Self {
            next: if index + 1 < POOL_SIZE {
                NodeIndex::new((index + 1) as u8)
            } else {
                None
            },
        }
    }
}

pub struct ItemHandle<'p, T: Reset, M: RawMutex, const W: usize> {
    pool: &'p PoolView<'p, T, M, W>,
    index: NodeIndex,
    _item: PhantomData<T>,
    _not_sync: PhantomData<Cell<()>>,
}

impl<'p, T: Reset, M: RawMutex, const W: usize> Deref for ItemHandle<'p, T, M, W> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: 슬롯을 소유하고, 참조 수명은 핸들 대여에 묶인다.
        unsafe { &*self.pool.data_ptr(self.index) }
    }
}

impl<'p, T: Reset, M: RawMutex, const W: usize> DerefMut for ItemHandle<'p, T, M, W> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: 핸들과 슬롯을 독점 접근한다.
        unsafe { &mut *self.pool.data_ptr(self.index) }
    }
}

impl<'p, T: Reset, M: RawMutex, const W: usize> Drop for ItemHandle<'p, T, M, W> {
    fn drop(&mut self) {
        // SAFETY: 소유한 슬롯을 한 번 반환한다.
        unsafe { self.pool.release_index(self.index) }
    }
}

struct IndexList {
    head: Option<NodeIndex>,
    tail: Option<NodeIndex>,
}

impl IndexList {
    const fn new() -> Self {
        Self {
            head: None,
            tail: None,
        }
    }

    // SAFETY: 리스트 소속 노드를 독점 접근할 수 있어야 한다.
    unsafe fn pop(&mut self, nodes: &[UnsafeCell<Node>]) -> Option<NodeIndex> {
        let index = self.head?;

        let node = unsafe { &mut *nodes[index.get() as usize].get() };

        self.head = node.next.take();

        if self.head.is_none() {
            self.tail = None;
        }

        Some(index)
    }

    // SAFETY: index는 이 리스트에 아직 속하지 않은 독점 소유 노드이고,
    // 리스트 소속 노드도 독점 접근할 수 있어야 한다.
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
    }
}

pub(crate) struct Sender<'q, 'p, T, M: RawMutex, const W: usize> {
    queue: &'q PoolLinkedList<'p, T, M, W>,
}

impl<'q, 'p, T: Reset, M: RawMutex, const W: usize> Sender<'q, 'p, T, M, W> {
    pub(crate) fn send(&self, item: ItemHandle<'p, T, M, W>) {
        self.queue.push(item);
    }

    pub(crate) fn try_acquire(&self) -> Option<ItemHandle<'p, T, M, W>> {
        self.queue.try_acquire()
    }

    pub(crate) async fn acquire(&self) -> Result<ItemHandle<'p, T, M, W>, WaitQueueFull> {
        self.queue.acquire().await
    }
}

pub(crate) struct Receiver<'q, 'p, T, M: RawMutex, const W: usize> {
    queue: &'q PoolLinkedList<'p, T, M, W>,
}

impl<'q, 'p, T: Reset, M: RawMutex, const W: usize> Receiver<'q, 'p, T, M, W> {
    pub(crate) fn try_receive(&self) -> Option<ItemHandle<'p, T, M, W>> {
        self.queue.try_pop()
    }

    pub(crate) async fn receive(&self) -> ItemHandle<'p, T, M, W> {
        self.queue.pop().await
    }
}

struct ListState {
    list: IndexList,
    waker: WakerRegistration,
}

pub(crate) struct PoolLinkedList<'p, T, M: RawMutex, const W: usize> {
    pool: &'p PoolView<'p, T, M, W>,
    state: Mutex<M, RefCell<ListState>>,
}

impl<'p, T: Reset, M: RawMutex, const W: usize> PoolLinkedList<'p, T, M, W> {
    pub(crate) fn new(pool: &'p PoolView<'p, T, M, W>) -> Self {
        Self {
            pool: pool,
            state: Mutex::new(RefCell::new(ListState {
                list: IndexList::new(),
                waker: WakerRegistration::new(),
            })),
        }
    }

    pub(crate) fn try_pop(&self) -> Option<ItemHandle<'p, T, M, W>> {
        let index = self.state.lock(|state| {
            let mut state = state.borrow_mut();

            unsafe { state.list.pop(self.pool.nodes) }
        })?;

        Some(ItemHandle {
            pool: self.pool,
            index,
            _item: PhantomData,
            _not_sync: PhantomData,
        })
    }

    pub(crate) async fn pop(&self) -> ItemHandle<'p, T, M, W> {
        let index = poll_fn(|cx| {
            self.state.lock(|state| {
                let mut state = state.borrow_mut();

                // SAFETY: 리스트 소속 노드는 이 잠금으로 독점 접근한다.
                if let Some(index) = unsafe { state.list.pop(self.pool.nodes) } {
                    Poll::Ready(index)
                } else {
                    state.waker.register(cx.waker());
                    Poll::Pending
                }
            })
        })
        .await;

        ItemHandle {
            pool: self.pool,
            index,
            _item: PhantomData,
            _not_sync: PhantomData,
        }
    }

    pub(crate) fn push(&self, item: ItemHandle<'p, T, M, W>) {
        assert!(
            core::ptr::eq(self.pool, item.pool),
            "handle belongs to a different pool"
        );

        let index = ManuallyDrop::new(item).index;

        self.state.lock(|state| {
            let mut state = state.borrow_mut();

            // SAFETY: 같은 풀의 핸들을 소비하여 노드를 리스트에 넘긴다.
            // 리스트 소속 노드는 이 잠금으로 보호한다.
            unsafe {
                state.list.push(self.pool.nodes, index);
            }

            state.waker.wake();
        });
    }

    pub(crate) fn try_acquire(&self) -> Option<ItemHandle<'p, T, M, W>> {
        self.pool.try_acquire()
    }

    pub(crate) async fn acquire(&self) -> Result<ItemHandle<'p, T, M, W>, WaitQueueFull> {
        self.pool.acquire().await
    }

    pub(crate) fn sender(&self) -> Sender<'_, 'p, T, M, W> {
        Sender { queue: self }
    }

    pub(crate) fn receiver(&self) -> Receiver<'_, 'p, T, M, W> {
        Receiver { queue: self }
    }
}

pub struct PoolStorage<T, M: RawMutex, const POOL_SIZE: usize, const WAITERS: usize> {
    data: [UnsafeCell<T>; POOL_SIZE],
    nodes: [UnsafeCell<Node>; POOL_SIZE],

    free_list: Mutex<M, RefCell<IndexList>>,
    waiters: FairSemaphore<M, WAITERS>,
}

impl<T: Reset, M, const POOL_SIZE: usize, const WAITERS: usize>
    PoolStorage<T, M, POOL_SIZE, WAITERS>
where
    M: RawMutex,
{
    pub fn new(mut init: impl FnMut() -> T) -> Self {
        const {
            assert!(WAITERS > 0, "waiters must be greater than 0");
            assert!(
                POOL_SIZE <= NodeIndex::MAX.get() as usize + 1,
                "invalid POOL_SIZE"
            );
        }

        let nodes: [UnsafeCell<Node>; POOL_SIZE] =
            core::array::from_fn(|index| UnsafeCell::new(Node::new::<POOL_SIZE>(index)));

        let free_list = IndexList {
            head: if POOL_SIZE > 0 {
                NodeIndex::new(0)
            } else {
                None
            },
            tail: POOL_SIZE
                .checked_sub(1)
                .and_then(|index| NodeIndex::new(index as u8)),
        };

        Self {
            data: core::array::from_fn(|_| UnsafeCell::new(init())),
            nodes: nodes,
            free_list: Mutex::new(RefCell::new(free_list)),
            waiters: FairSemaphore::new(POOL_SIZE),
        }
    }

    pub fn view(&mut self) -> PoolView<'_, T, M, WAITERS> {
        PoolView {
            data: &self.data,
            nodes: &self.nodes,
            free_list: &self.free_list,
            waiters: &self.waiters,
        }
    }
}

unsafe impl<T: Send, M: RawMutex + Sync, const POOL_SIZE: usize, const WAITERS: usize> Sync
    for PoolStorage<T, M, POOL_SIZE, WAITERS>
{
}

pub struct PoolView<'a, T, M: RawMutex, const W: usize> {
    data: &'a [UnsafeCell<T>],
    nodes: &'a [UnsafeCell<Node>],
    free_list: &'a Mutex<M, RefCell<IndexList>>,
    waiters: &'a FairSemaphore<M, W>,
}

unsafe impl<'a, T: Send, M: RawMutex + Sync, const W: usize> Sync for PoolView<'a, T, M, W> {}

impl<'a, T: Reset, M: RawMutex, const W: usize> PoolView<'a, T, M, W> {
    pub(crate) fn try_acquire(&self) -> Option<ItemHandle<'_, T, M, W>> {
        let permit = self.waiters.try_acquire(1)?;
        let item = self.get_item();

        permit.disarm();
        Some(item)
    }

    pub(crate) async fn acquire(&self) -> Result<ItemHandle<'_, T, M, W>, WaitQueueFull> {
        let permit = self.waiters.acquire(1).await?;
        let item = self.get_item();

        permit.disarm();
        Ok(item)
    }

    fn get_item(&self) -> ItemHandle<'_, T, M, W> {
        let index = self
            .free_list
            .lock(|state| {
                let mut free_list = state.borrow_mut();

                // SAFETY: free list 소속 노드는 이 잠금으로 보호한다.
                unsafe { free_list.pop(&self.nodes) }
            })
            .expect("semaphore acquired but empty list?");

        ItemHandle {
            pool: self,
            index,
            _item: PhantomData,
            _not_sync: PhantomData,
        }
    }

    //     pub(crate) fn release(&self, item: ItemHandle<'_, T, M, W>) {
    //         assert!(
    //             core::ptr::eq(self, item.pool),
    //             "handle belongs to a different pool"
    //         );
    //
    //         drop(item);
    //     }

    // pub(crate) fn list(&self) -> PoolLinkedList<'_, T, M, W> {
    //     PoolLinkedList::new(self)
    // }

    fn data_ptr(&self, index: NodeIndex) -> *mut T {
        self.data[index.get() as usize].get()
    }

    // SAFETY: 호출자는 해당 슬롯을 독점 소유하며,
    // 반환 후 슬롯에 접근하지 않아야 한다.
    unsafe fn release_index(&self, index: NodeIndex) {
        unsafe {
            (&mut *self.data[index.get() as usize].get()).clear();
        }

        self.free_list.lock(|state| unsafe {
            state.borrow_mut().push(self.nodes, index);
        });

        self.waiters.release(1);
    }
}

pub trait Reset {
    fn clear(&mut self);
}
