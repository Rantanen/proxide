using Google.Protobuf;

using DotNet.Performance;

namespace DotNet.Performance
{
    partial class Blob {
        public Blob(long size) {
            for (long i1 = 0; i1 < size; i1++ )
            {
                var item = new BlobItem();
                for (long i2 = 0; i2 < size; i2++ )
                {
                    var sub = new BlobSubItem();
                    for (long i3 = 0; i3 < size; i3++ )
                    {
                        sub.Data.Add(ByteString.CopyFrom(new byte[size]));
                    }
                    item.SubItems.Add(sub);
                }
                this.Items.Add(item);
            }
        }
    }
}
