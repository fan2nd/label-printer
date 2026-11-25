// Classic Bluetooth Serial Port Profile transport. All IOBluetooth objects
// and delegate callbacks belong to the main run loop.
#import <IOBluetooth/IOBluetooth.h>

@interface LPBrotherChannel : NSObject <IOBluetoothRFCOMMChannelDelegate>
@property(strong) IOBluetoothDevice *device;
@property(strong) IOBluetoothRFCOMMChannel *channel;
@property(strong) NSMutableData *input;
@property BOOL awaitingService;
@property int state; // 0 connecting, 1 open, -1 closed or failed
@end

@implementation LPBrotherChannel
- (void)openSerialService {
    IOBluetoothSDPServiceRecord *record = [_device getServiceRecordForUUID:[IOBluetoothSDPUUID uuid16:0x1101]];
    BluetoothRFCOMMChannelID number = 0;
    IOBluetoothRFCOMMChannel *channel = nil;
    if (!record || [record getRFCOMMChannelID:&number] != kIOReturnSuccess ||
        [_device openRFCOMMChannelAsync:&channel withChannelID:number delegate:self] != kIOReturnSuccess) {
        _state = -1;
        return;
    }
    _channel = channel;
    // openRFCOMMChannelAsync returns a retained out parameter.
    CFRelease((__bridge CFTypeRef)channel);
}
- (void)rfcommChannelOpenComplete:(IOBluetoothRFCOMMChannel *)channel status:(IOReturn)status {
    _state = status == kIOReturnSuccess ? 1 : -1;
}
- (void)rfcommChannelClosed:(IOBluetoothRFCOMMChannel *)channel { _state = -1; }
- (void)rfcommChannelData:(IOBluetoothRFCOMMChannel *)channel data:(void *)dataPointer length:(size_t)dataLength {
    [_input appendBytes:dataPointer length:dataLength];
}
@end

static void onMain(dispatch_block_t action) {
    if ([NSThread isMainThread]) action();
    else dispatch_sync(dispatch_get_main_queue(), action);
}

void *lp_brother_open(int *count) {
    __block void *result = NULL;
    onMain(^{
        NSMutableArray *matches = [NSMutableArray array];
        for (IOBluetoothDevice *d in [IOBluetoothDevice pairedDevices]) {
            if ([d.name.uppercaseString hasPrefix:@"PT-P300BT"]) [matches addObject:d];
        }
        *count = (int)matches.count;
        if (*count != 1) return;
        LPBrotherChannel *session = [LPBrotherChannel new];
        session.input = [NSMutableData data];
        session.device = matches.firstObject;
        if ([session.device getServiceRecordForUUID:[IOBluetoothSDPUUID uuid16:0x1101]])
            [session openSerialService];
        else {
            session.awaitingService = YES;
            if ([session.device performSDPQuery:nil] != kIOReturnSuccess) session.state = -1;
        }
        result = (__bridge_retained void *)session;
    });
    return result;
}

int lp_brother_state(void *handle) {
    __block int state;
    onMain(^{
        LPBrotherChannel *s = (__bridge LPBrotherChannel *)handle;
        if (s.state == 0 && s.awaitingService &&
            [s.device getServiceRecordForUUID:[IOBluetoothSDPUUID uuid16:0x1101]]) {
            s.awaitingService = NO;
            [s openSerialService];
        }
        state = s.state;
    });
    return state;
}

int lp_brother_write(void *handle, const unsigned char *bytes, int length) {
    __block int sent = -1;
    onMain(^{
        LPBrotherChannel *s = (__bridge LPBrotherChannel *)handle;
        if (s.state != 1) return;
        int n = MIN(length, [s.channel getMTU]);
        if (n > 0 && [s.channel writeSync:(void *)bytes length:n] == kIOReturnSuccess) sent = n;
    });
    return sent;
}

int lp_brother_status(void *handle, unsigned char *frame) {
    __block int result = 0;
    onMain(^{
        LPBrotherChannel *s = (__bridge LPBrotherChannel *)handle;
        if (s.input.length >= 32) {
            [s.input getBytes:frame length:32];
            [s.input replaceBytesInRange:NSMakeRange(0,32) withBytes:NULL length:0];
            result = 1;
        } else if (s.state < 0) result = -1;
    });
    return result;
}

void lp_brother_close(void *handle) {
    onMain(^{
        LPBrotherChannel *s = (__bridge_transfer LPBrotherChannel *)handle;
        s.state = -1;
        [s.channel setDelegate:nil];
        [s.channel closeChannel];
    });
}
